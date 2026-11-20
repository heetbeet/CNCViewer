use super::*;
fn reference_end(text: &str, start: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    let mut end = start + 1;
    while bytes.get(end) == Some(&b'#') {
        end += 1;
    }
    if end - start > 128 {
        return Err(NcError::new("DPRNT variable indirection exceeds 128 levels").code("LIMIT"));
    }
    if bytes.get(end) == Some(&b'[') {
        let mut depth = 1;
        end += 1;
        while end < bytes.len() {
            if bytes[end] == b'[' {
                depth += 1;
            }
            if bytes[end] == b']' {
                depth -= 1;
                if depth == 0 {
                    return Ok(end + 1);
                }
            }
            end += 1;
        }
        return Err(NcError::new("Unclosed DPRNT variable address"));
    }
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == start + 1 {
        return Err(NcError::new("DPRNT requires a variable address after #"));
    }
    Ok(end)
}
impl Engine {
    pub(super) fn print(&mut self, command: &str, text: &str) -> Result<()> {
        if command != "DPRNT" {
            self.event(
                if command == "POPEN" { "popen" } else { "pclos" },
                json!({}),
            );
            return Ok(());
        }
        let mut result = String::new();
        let mut index = 0;
        while index < text.len() {
            if text.as_bytes()[index] != b'#' {
                let c = text[index..].chars().next().unwrap();
                result.push(if c == '*' { ' ' } else { c });
                index += c.len_utf8();
                continue;
            }
            let end = reference_end(text, index)?;
            let block = crate::language::parse_block(&format!("#99999={}", &text[index..end]));
            if !block["error"].is_null() {
                return Err(NcError::new("Invalid DPRNT variable reference"));
            }
            let value = self.evaluate(&block["ok"]["items"][0]["value"])?;
            index = end;
            let rest = &text.as_bytes()[index..];
            let format = rest.len() >= 4
                && rest[0] == b'['
                && rest[1].is_ascii_digit()
                && rest[2].is_ascii_digit()
                && rest[3] == b']';
            if format {
                index += 4;
            }
            if let Some(value) = value {
                if format {
                    let decimals = (rest[2] - b'0') as usize;
                    let width =
                        (rest[1] - b'0') as usize + if decimals > 0 { decimals + 1 } else { 0 };
                    result.push_str(&format!("{value:width$.decimals$}"));
                } else {
                    result.push_str(&value.to_string());
                }
            }
        }
        self.event("dprnt", json!({"text":result}));
        Ok(())
    }
}
