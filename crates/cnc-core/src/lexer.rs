use serde_json::{json, Value};
use std::borrow::Cow;
const KEYWORDS: &[&str] = &[
    "SETVN", "WHILE", "PCLOS", "POPEN", "DPRNT", "GOTO", "THEN", "ROUND", "SQRT", "ASIN", "ACOS",
    "ATAN", "ABS", "FIX", "FUP", "SIN", "COS", "TAN", "EXP", "MOD", "XOR", "AND", "END", "BCD",
    "BIN", "PRM", "NOT", "DO", "IF", "EQ", "NE", "LT", "LE", "GT", "GE", "OR", "LN",
];
fn whitespace(c: char) -> bool {
    matches!(c,'\u{9}'..='\u{d}'|' '| '\u{a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn diagnostic(message: impl Into<String>, column: Option<usize>, limit: bool) -> Value {
    let mut value = json!({"message":message.into(),"code":if limit{"LIMIT"}else{"PROGRAM"}});
    if let Some(column) = column {
        value["column"] = json!(column)
    }
    value
}
pub fn uncomment(text: &str) -> Value {
    match strip_comments(text) {
        Ok((code, comments)) => json!({"ok":{"code":code,"comments":comments}}),
        Err(error) => json!({"error":error}),
    }
}

pub(crate) fn strip_comments(text: &str) -> Result<(Cow<'_, str>, Vec<String>), Value> {
    if text.bytes().all(|byte| {
        byte.is_ascii() && !byte.is_ascii_lowercase() && !matches!(byte, b'(' | b')' | b';')
    }) {
        return Ok((Cow::Borrowed(text), Vec::new()));
    }
    let mut code = String::new();
    let mut comment = String::new();
    let mut comments = Vec::new();
    let mut depth = 0usize;
    let mut column = 0;
    for (offset, c) in text.char_indices() {
        match c {
            '(' => {
                if depth == 0 {
                    comment.clear()
                }
                depth += 1;
                code.push(' ')
            }
            ')' => {
                if depth == 0 {
                    return Err(diagnostic(
                        "Unmatched closing comment parenthesis.",
                        Some(column),
                        false,
                    ));
                }
                depth -= 1;
                code.push(' ');
                if depth == 0 {
                    comments.push(std::mem::take(&mut comment))
                }
            }
            _ if depth > 0 => {
                comment.push(c);
                for _ in 0..c.len_utf16() {
                    code.push(' ')
                }
            }
            ';' => {
                comments.push(text[offset + 1..].to_owned());
                break;
            }
            _ => code.push(c),
        }
        column += c.len_utf16();
    }
    if depth > 0 {
        return Err(diagnostic("Unclosed comment.", None, false));
    }
    Ok((Cow::Owned(code.to_uppercase()), comments))
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Number,
    Named,
    Keyword,
    Address,
    Operator,
    End,
}

#[derive(Clone, Copy)]
pub(crate) struct Token<'a> {
    pub kind: Kind,
    pub text: &'a str,
    pub number: f64,
    pub column: usize,
}

impl<'a> Token<'a> {
    fn symbol_token(kind: Kind, text: &'a str, column: usize) -> Self {
        Self {
            kind,
            text,
            number: 0.,
            column,
        }
    }
    pub fn symbol(&self) -> &'a str {
        if self.kind == Kind::Number {
            ""
        } else {
            self.text
        }
    }
    pub fn value(&self) -> Value {
        if self.kind == Kind::Number {
            Value::from(self.number)
        } else {
            Value::from(self.text)
        }
    }
}

pub fn lex(code: &str) -> Value {
    let tokens = match tokenize(code) {
        Ok(tokens) => tokens,
        Err(error) => return json!({"error":error}),
    };
    let tokens = tokens
        .into_iter()
        .map(|token| {
            let kind = match token.kind {
                Kind::Number => "number",
                Kind::Named => "named",
                Kind::Keyword => "keyword",
                Kind::Address => "address",
                Kind::Operator => "operator",
                Kind::End => "end",
            };
            let mut value = json!({"kind":kind,"value":token.value(),"column":token.column});
            if token.kind == Kind::Number {
                value["raw"] = Value::from(token.text);
            }
            value
        })
        .collect();
    Value::Object(serde_json::Map::from_iter([(
        "ok".into(),
        Value::Array(tokens),
    )]))
}

pub(crate) fn tokenize(code: &str) -> Result<Vec<Token<'_>>, Value> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut column = 0;
    let mut depth = 0;
    while offset < code.len() {
        if tokens.len() > 4096 {
            return Err(diagnostic(
                "A block exceeds the token limit.",
                Some(column),
                true,
            ));
        }
        let rest = &code[offset..];
        let c = rest.chars().next().unwrap();
        if whitespace(c) {
            offset += c.len_utf8();
            column += c.len_utf16();
            continue;
        }
        let mut count = 0;
        let bytes = rest.as_bytes();
        while count < bytes.len() && bytes[count].is_ascii_digit() {
            count += 1
        }
        let digits = count;
        if count < bytes.len() && bytes[count] == b'.' {
            count += 1;
            while count < bytes.len() && bytes[count].is_ascii_digit() {
                count += 1
            }
        }
        if digits > 0 || (count > 1 && bytes[0] == b'.') {
            let raw = &rest[..count];
            let value = raw.parse::<f64>().unwrap_or(f64::INFINITY);
            if !value.is_finite() {
                return Err(diagnostic(
                    "Numeric value is outside the supported range.",
                    Some(column),
                    false,
                ));
            }
            tokens.push(Token {
                kind: Kind::Number,
                text: raw,
                number: value,
                column,
            });
            offset += count;
            column += count;
            continue;
        }
        if c == '#'
            && bytes
                .get(1)
                .is_some_and(|b| b.is_ascii_uppercase() || *b == b'_')
        {
            count = 2;
            while count < bytes.len()
                && (bytes[count].is_ascii_uppercase()
                    || bytes[count].is_ascii_digit()
                    || bytes[count] == b'_')
            {
                count += 1
            }
            tokens.push(Token::symbol_token(Kind::Named, &rest[1..count], column));
            offset += count;
            column += count;
            continue;
        }
        let abbreviation =
            if bytes.len() >= 2 && bytes[0].is_ascii_uppercase() && bytes[1].is_ascii_uppercase() {
                match &rest[..2] {
                    "AB" => Some("ABS"),
                    "RO" => Some("ROUND"),
                    "FI" => Some("FIX"),
                    "FU" => Some("FUP"),
                    "SI" => Some("SIN"),
                    "CO" => Some("COS"),
                    "TA" => Some("TAN"),
                    "AS" => Some("ASIN"),
                    "AC" => Some("ACOS"),
                    "AT" => Some("ATAN"),
                    "SQ" => Some("SQRT"),
                    "EX" => Some("EXP"),
                    "BI" => Some("BIN"),
                    "BC" => Some("BCD"),
                    "PR" => Some("PRM"),
                    _ => None,
                }
            } else {
                None
            };
        if let Some(name) = abbreviation {
            count = 2;
            while count < rest.len() {
                let c = rest[count..].chars().next().unwrap();
                if !whitespace(c) {
                    break;
                }
                count += c.len_utf8()
            }
            if rest[count..].starts_with('[') {
                tokens.push(Token::symbol_token(Kind::Keyword, name, column));
                column += rest[..count].encode_utf16().count();
                offset += count;
                continue;
            }
        }
        if let Some(word) = KEYWORDS.iter().find(|word| rest.starts_with(**word)) {
            tokens.push(Token::symbol_token(Kind::Keyword, word, column));
            offset += word.len();
            column += word.len();
            continue;
        }
        if c.is_ascii_uppercase() {
            tokens.push(Token::symbol_token(Kind::Address, &rest[..1], column));
            offset += 1;
            column += 1;
            continue;
        }
        let operator = ["**", "<>", "<=", ">=", "=="]
            .into_iter()
            .find(|op| rest.starts_with(op));
        let operator = operator.or_else(|| {
            if "+*/-=<>#[],%:".contains(c) {
                Some(&rest[..1])
            } else {
                None
            }
        });
        if let Some(op) = operator {
            if op == "[" {
                depth += 1;
                if depth > 10 {
                    return Err(diagnostic(
                        "MachPro expressions allow at most ten bracket levels.",
                        Some(column),
                        false,
                    ));
                }
            }
            if op == "]" {
                depth -= 1;
                if depth < 0 {
                    return Err(diagnostic(
                        "Unmatched closing expression bracket.",
                        Some(column),
                        false,
                    ));
                }
            }
            offset += op.len();
            tokens.push(Token::symbol_token(Kind::Operator, op, column));
            column += op.len();
            continue;
        }
        return Err(diagnostic(
            format!("Unexpected character '{c}'."),
            Some(column),
            false,
        ));
    }
    if depth != 0 {
        return Err(diagnostic(
            "Unclosed expression bracket.",
            Some(column),
            false,
        ));
    }
    tokens.push(Token::symbol_token(Kind::End, "", column));
    Ok(tokens)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_comments_columns_and_abbreviations() {
        assert_eq!(uncomment("(🛠) G1; hi")["ok"]["code"], "     G1");
        let tokens = lex("G1X#100Y-SI [30]");
        assert_eq!(tokens["ok"][7]["value"], "SIN");
        assert_eq!(uncomment("(open")["error"]["message"], "Unclosed comment.");
    }
    #[test]
    fn bounds() {
        assert_eq!(lex("[[[[[[[[[[[1]]]]]]]]]]]")["error"]["column"], 10);
        assert!(lex(&"X1".repeat(3000))["error"].is_object());
        assert!(lex(".")["error"].is_object());
    }
}
