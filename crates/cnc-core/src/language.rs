use serde_json::{json, Value};
type Result<T> = std::result::Result<T, Value>;
fn failure(message: impl Into<String>, column: Option<usize>) -> Value {
    let mut e = json!({"message":message.into(),"code":"PROGRAM"});
    if let Some(column) = column {
        e["column"] = json!(column)
    }
    e
}
fn integer(value: f64, label: &str, min: f64, max: f64) -> Result<f64> {
    if value.is_finite() && value.fract() == 0. && value >= min && value <= max {
        Ok(value)
    } else {
        Err(failure(
            format!("{label} must be an integer between {min} and {max}."),
            None,
        ))
    }
}
struct Parser {
    tokens: Vec<Value>,
    index: usize,
    depth: usize,
}
impl Parser {
    fn bounded<T>(&mut self, action: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.depth >= 128 {
            return Err(
                json!({"message":"Native syntax nesting exceeds 128 levels.","code":"LIMIT","column":self.tokens[self.index]["column"]}),
            );
        }
        self.depth += 1;
        let result = action(self);
        self.depth -= 1;
        result
    }
    fn value(&self) -> &str {
        self.tokens[self.index]["value"].as_str().unwrap_or("")
    }
    fn kind(&self) -> &str {
        self.tokens[self.index]["kind"].as_str().unwrap_or("")
    }
    fn done(&self) -> bool {
        self.kind() == "end"
    }
    fn take(&mut self, value: &str) -> bool {
        if self.value() == value {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn error(&self, message: impl Into<String>) -> Value {
        failure(
            message,
            self.tokens[self.index]["column"]
                .as_u64()
                .map(|v| v as usize),
        )
    }
    fn expect(&mut self, value: &str) -> Result<()> {
        if self.take(value) {
            Ok(())
        } else {
            Err(self.error(format!(
                "Expected '{value}', found '{}'.",
                if self.done() {
                    "end of block"
                } else {
                    self.value()
                }
            )))
        }
    }
    fn next(&mut self) -> Value {
        let value = self.tokens[self.index].clone();
        self.index = (self.index + 1).min(self.tokens.len() - 1);
        value
    }
    fn variable(&mut self) -> Result<Value> {
        self.bounded(|parser| parser.variable_inner())
    }
    fn variable_inner(&mut self) -> Result<Value> {
        if self.kind() == "named" {
            return Ok(json!({"type":"named","name":self.next()["value"]}));
        }
        self.expect("#")?;
        if self.take("[") {
            let address = self.expression(0)?;
            self.expect("]")?;
            return Ok(json!({"type":"variable","address":address}));
        }
        if self.value() == "#" || self.kind() == "named" {
            let address = self.variable()?;
            return Ok(json!({"type":"variable","address":address}));
        }
        if self.kind() != "number" {
            return Err(
                self.error("Expected a variable number, name or bracketed address after '#'.")
            );
        }
        Ok(json!({"type":"variable","address":{"type":"number","value":self.next()["value"]}}))
    }
    fn expression(&mut self, minimum: u32) -> Result<Value> {
        self.bounded(|parser| parser.expression_inner(minimum))
    }
    fn expression_inner(&mut self, minimum: u32) -> Result<Value> {
        let left = if self.kind() == "number" {
            json!({"type":"number","value":self.next()["value"]})
        } else if self.value() == "#" || self.kind() == "named" {
            self.variable()?
        } else if self.take("[") {
            let value = self.expression(0)?;
            self.expect("]")?;
            value
        } else if ["+", "-", "NOT"].contains(&self.value()) {
            let operator = self.next()["value"].clone();
            json!({"type":"unary","operator":operator,"operand":self.expression(40)?})
        } else if [
            "ABS", "ROUND", "FIX", "FUP", "SIN", "COS", "TAN", "ASIN", "ACOS", "ATAN", "SQRT",
            "LN", "EXP", "BIN", "BCD", "PRM",
        ]
        .contains(&self.value())
        {
            let name = self.next()["value"].clone();
            self.expect("[")?;
            let mut args = vec![self.expression(0)?];
            while self.take(",") {
                args.push(self.expression(0)?)
            }
            self.expect("]")?;
            if name == "ATAN"
                && self.value() == "/"
                && self
                    .tokens
                    .get(self.index + 1)
                    .is_some_and(|t| t["value"] == "[")
            {
                self.next();
                self.expect("[")?;
                args.push(self.expression(0)?);
                self.expect("]")?
            }
            json!({"type":"function","name":name,"args":args})
        } else {
            return Err(self.error(format!(
                "Expected a number or expression, found '{}'.",
                if self.done() {
                    "end of block"
                } else {
                    self.value()
                }
            )));
        };
        let mut operations = Vec::new();
        while !self.done() {
            let operator = self.value().to_owned();
            let precedence = match operator.as_str() {
                "EQ" | "NE" | "LT" | "LE" | "GT" | "GE" | "=" | "==" | "<>" | "<" | "<=" | ">"
                | ">=" => 10,
                "+" | "-" | "OR" | "XOR" => 20,
                "*" | "/" | "MOD" | "AND" => 30,
                "**" => 50,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.next();
            let right = self.expression(precedence + u32::from(operator != "**"))?;
            operations.push(Value::Object(serde_json::Map::from_iter([
                ("operator".into(), Value::String(operator)),
                ("operand".into(), right),
            ])));
        }
        if operations.is_empty() {
            Ok(left)
        } else {
            Ok(Value::Object(serde_json::Map::from_iter([
                ("type".into(), Value::String("chain".into())),
                ("first".into(), left),
                ("operations".into(), Value::Array(operations)),
            ])))
        }
    }
    fn statements(&mut self) -> Result<Vec<Value>> {
        self.bounded(|parser| parser.statements_inner())
    }
    fn statements_inner(&mut self) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        while !self.done() {
            if self.take("%") {
                continue;
            }
            if self.take("IF") {
                self.expect("[")?;
                let condition = self.expression(0)?;
                self.expect("]")?;
                self.take("THEN");
                let consequent = self.statements()?;
                if consequent.is_empty() {
                    return Err(self.error("IF requires a statement or GOTO target."));
                }
                items.push(json!({"type":"if","condition":condition,"consequent":consequent}));
            } else if self.take("GOTO") {
                items.push(json!({"type":"goto","target":self.expression(0)?}));
            } else if self.take("WHILE") {
                self.expect("[")?;
                let condition = self.expression(0)?;
                self.expect("]")?;
                self.expect("DO")?;
                let id = self.next();
                if id["kind"] != "number" {
                    return Err(self.error("DO requires a loop number."));
                }
                let id = integer(
                    id["value"].as_f64().unwrap(),
                    "Loop identifier",
                    1.,
                    99999999.,
                )?;
                items.push(json!({"type":"while","condition":condition,"id":id}));
            } else if self.take("END") {
                let id = self.next();
                if id["kind"] != "number" {
                    return Err(self.error("END requires a loop number."));
                }
                let id = integer(
                    id["value"].as_f64().unwrap(),
                    "Loop identifier",
                    1.,
                    99999999.,
                )?;
                items.push(json!({"type":"end","id":id}));
            } else if self.value() == "#" || self.kind() == "named" {
                let target = self.variable()?;
                self.expect("=")?;
                items.push(json!({"type":"assign","target":target,"value":self.expression(0)?}));
            } else if self.kind() == "address" || self.value() == ":" {
                let letter = self.next()["value"].as_str().unwrap().to_owned();
                let start = self.index;
                if self.done() || self.kind() == "address" {
                    let address = if letter == ":" { "O" } else { &letter };
                    return Err(self.error(format!("Address {address} is missing a value")));
                }
                let value = self.expression(0)?;
                let literal = &self.tokens[start..self.index];
                let raw = match literal {
                    [number] => number["kind"] == "number",
                    [sign, number] => {
                        (sign["value"] == "+" || sign["value"] == "-") && number["kind"] == "number"
                    }
                    _ => false,
                };
                let decimal = raw
                    && literal
                        .iter()
                        .any(|t| t["raw"].as_str().is_some_and(|v| v.contains('.')));
                let text = if raw {
                    json!(literal
                        .iter()
                        .map(|t| t["raw"]
                            .as_str()
                            .or_else(|| t["value"].as_str())
                            .unwrap_or(""))
                        .collect::<String>())
                } else {
                    Value::Null
                };
                items.push(json!({"type":"word","letter":if letter==":"{"O"}else{&letter},"value":value,"decimalLiteral":decimal,"rawLiteral":text}));
            } else {
                return Err(self.error(format!("Unexpected '{}' in a block.", self.value())));
            }
        }
        Ok(items)
    }
}
pub fn parse_block(text: &str) -> Value {
    match parse_syntax(text) {
        Ok(block) => Value::Object(serde_json::Map::from_iter([("ok".into(), block)])),
        Err(error) => json!({"error":error}),
    }
}
pub(crate) fn parse_syntax(text: &str) -> Result<Value> {
    let uncomment = super::lexer::uncomment(text);
    if uncomment.get("error").is_some() {
        return Err(uncomment["error"].clone());
    }
    let mut active = uncomment["ok"]["code"].as_str().unwrap().trim().to_owned();
    let mut deleted = Value::Null;
    if active.starts_with('/') {
        let tail = &active[1..];
        let digit = tail.as_bytes().first().filter(|c| c.is_ascii_digit());
        deleted = json!(digit.map_or(0, |c| (c - b'0') as u32));
        active = tail[usize::from(digit.is_some())..].trim_start().to_owned();
    }
    let prefix = if let Some(tail) = active.strip_prefix('N') {
        tail.bytes().take_while(u8::is_ascii_digit).count()
    } else {
        0
    };
    let special = if prefix > 0 {
        active[prefix + 1..].trim_start()
    } else {
        &active
    };
    let mut items = None;
    if let Some(tail) = special.strip_prefix("SETVN") {
        let tail = tail.trim_start();
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            let names = tail[digits..].trim();
            if names.starts_with('[')
                && names.ends_with(']')
                && !names[1..names.len() - 1].contains(']')
                && names.len() > 2
            {
                let start = integer(
                    tail[..digits].parse().unwrap_or(f64::INFINITY),
                    "SETVN starting variable",
                    500.,
                    999.,
                )?;
                items = Some(vec![
                    json!({"type":"names","start":start,"names":names[1..names.len()-1].split(',').map(|n|n.trim().to_uppercase()).collect::<Vec<_>>()}),
                ]);
            }
        }
    }
    for command in ["DPRNT", "POPEN", "PCLOS"] {
        if let Some(tail) = special.strip_prefix(command) {
            let tail = tail.trim();
            if tail.is_empty() || (tail.starts_with('[') && tail.ends_with(']')) {
                items = Some(vec![
                    json!({"type":"print","command":command,"text":if tail.is_empty(){""}else{&tail[1..tail.len()-1]}}),
                ]);
            }
        }
    }
    let special = items.is_some();
    let mut items = match items {
        Some(items) => items,
        None => {
            let mut tokens = super::lexer::lex(&active);
            if tokens.get("error").is_some() {
                return Err(tokens["error"].clone());
            }
            Parser {
                tokens: std::mem::take(tokens["ok"].as_array_mut().unwrap()),
                index: 0,
                depth: 0,
            }
            .statements()?
        }
    };
    if special && prefix > 0 {
        items.insert(0,json!({"type":"word","letter":"N","value":{"type":"number","value":active[1..prefix+1].parse::<f64>().unwrap_or(f64::INFINITY)}}));
    }
    let mut label = Value::Null;
    let mut program = Value::Null;
    for (letter, description, slot) in [
        ("N", "Sequence", &mut label),
        ("O", "Program", &mut program),
    ] {
        if let Some(item) = items
            .iter()
            .find(|i| i["type"] == "word" && i["letter"] == letter)
        {
            if item["value"]["type"] != "number" {
                return Err(failure(
                    if letter == "N" {
                        "Sequence labels must be literal numbers."
                    } else {
                        "Program identifiers must be literal numbers."
                    },
                    None,
                ));
            }
            *slot = json!(integer(
                item["value"]["value"].as_f64().unwrap_or(f64::INFINITY),
                &format!("{description} number"),
                0.,
                99999999.
            )?);
        }
    }
    Ok(Value::Object(serde_json::Map::from_iter([
        ("comments".into(), uncomment["ok"]["comments"].clone()),
        ("items".into(), Value::Array(items)),
        ("blockDelete".into(), deleted),
        ("label".into(), label),
        ("program".into(), program),
    ])))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_statements_are_diagnostics() {
        for text in ["END", "WHILE[1]DO", "IF[1]", "#", "G1X", "N#1", "O#1"] {
            assert!(parse_block(text).get("error").is_some(), "{text}");
        }
    }
    #[test]
    fn all_statement_forms() {
        for text in [
            "N10 IF[#100 LT 10]GOTO20",
            "WHILE[#100 GT 0]DO1",
            "END1",
            "SETVN500[FOO,BAR]",
            "N10 DPRNT[VALUE #100[44]]",
            "#100=ATAN[1]/[-1]",
            "/2 N10 G1X[2**3]F100",
            ":1000",
        ] {
            assert!(parse_block(text).get("ok").is_some(), "{text}");
        }
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    #[test]
    fn excessive_nesting_is_bounded_and_parser_recovers() {
        for text in [
            format!("#100={}1", "-".repeat(4090)),
            format!("#100={}1", "-".repeat(500)),
            format!("{}G1X1", "IF[1]".repeat(200)),
        ] {
            assert_eq!(parse_block(&text)["error"]["code"], "LIMIT");
            assert!(parse_block("G1X1F100").get("ok").is_some());
        }
    }
}
