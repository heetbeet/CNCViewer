use crate::{
    error::{NcError, Result},
    language::parse_syntax,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
pub struct Block {
    pub source: String,
    pub line: usize,
    pub text: String,
    pub syntax: Value,
    pub error: Option<NcError>,
    pub(crate) linear: Option<LinearBlock>,
}
pub(crate) struct LinearBlock {
    pub code: [f64; 1],
    pub words: BTreeMap<char, f64>,
}

// Most posted cuts contain literal linear moves. Keep those compact and let
// the general parser handle comments, macros, labels and unusual syntax.
fn linear_block(text: &str) -> Option<LinearBlock> {
    let mut input = text.trim_ascii();
    if !input.starts_with(['G', 'g']) {
        return None;
    }
    let mut code = None;
    let mut words = BTreeMap::new();
    while !input.is_empty() {
        let letter = input.as_bytes()[0].to_ascii_uppercase();
        if !matches!(letter, b'G' | b'X' | b'Y' | b'Z' | b'F') {
            return None;
        }
        input = input[1..].trim_ascii_start();
        let length = input
            .bytes()
            .take_while(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
            .count();
        let value: f64 = input[..length].parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        if letter == b'G' {
            if ![0., 1.].contains(&value) || code.replace(value).is_some() {
                return None;
            }
        } else if words.insert(char::from(letter), value).is_some() {
            return None;
        }
        input = input[length..].trim_ascii_start();
    }
    Some(LinearBlock {
        code: [code?],
        words,
    })
}
pub struct Program {
    pub id: Option<u32>,
    pub source: String,
    pub blocks: Vec<Arc<Block>>,
    pub labels: BTreeMap<u32, usize>,
}
pub struct Library {
    pub programs: Vec<Program>,
    pub ids: BTreeMap<u32, usize>,
    pub sources: BTreeMap<String, usize>,
    pub variable_numbers: BTreeSet<u32>,
}
pub fn compile(
    text: &str,
    filename: &str,
    libraries: &BTreeMap<String, String>,
) -> Result<Library> {
    compile_cancellable(text, filename, libraries, None)
}
pub(crate) fn compile_cancellable(
    text: &str,
    filename: &str,
    libraries: &BTreeMap<String, String>,
    cancel: Option<&AtomicBool>,
) -> Result<Library> {
    let check = || {
        if cancel.is_some_and(|token| token.load(Ordering::Relaxed)) {
            Err(NcError::new("Plot cancelled").code("CANCELLED"))
        } else {
            Ok(())
        }
    };
    let mut library = Library {
        programs: Vec::new(),
        ids: BTreeMap::new(),
        sources: BTreeMap::new(),
        variable_numbers: BTreeSet::new(),
    };
    for (source, text) in std::iter::once((filename, text))
        .chain(libraries.iter().map(|(n, t)| (n.as_str(), t.as_str())))
    {
        if library.sources.contains_key(source) {
            return Err(NcError::new(format!("Duplicate source filename {source}.")));
        }
        let text = text
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let mut blocks: Vec<Block> = text
            .split('\n')
            .enumerate()
            .map(|(line, text)| {
                check()?;
                let linear = linear_block(text);
                let parsed = if linear.is_some() {
                    Ok(Value::Null)
                } else {
                    parse_syntax(text)
                };
                let (syntax, error) = match parsed {
                    Ok(syntax) => (syntax, None),
                    Err(e) => {
                        let mut error =
                            NcError::new(e["message"].as_str().unwrap_or("Invalid syntax"))
                                .code(e["code"].as_str().unwrap_or("PROGRAM"));
                        error.column = e["column"].as_u64().map(|n| n as usize);
                        (Value::Null, Some(error))
                    }
                };
                Ok(Block {
                    source: source.into(),
                    line: line + 1,
                    text: text.into(),
                    syntax,
                    error,
                    linear,
                })
            })
            .collect::<Result<_>>()?;
        let mut headers: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.syntax["program"].as_f64().map(|_| i))
            .collect();
        if headers.is_empty() {
            headers.push(0)
        } else if headers[0] > 0
            && blocks[..headers[0]].iter().any(|b| {
                b.syntax["items"].as_array().is_some_and(|items| {
                    items.iter().any(|i| {
                        i["letter"] == "M"
                            && i["value"]["type"] == "number"
                            && [2., 30., 99.].contains(&i["value"]["value"].as_f64().unwrap_or(-1.))
                    })
                })
            })
        {
            headers.insert(0, 0)
        }
        let file_id = source
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(source)
            .split('.')
            .next()
            .unwrap_or("")
            .trim_start_matches(['O', 'o'])
            .parse::<u32>()
            .ok();
        library
            .sources
            .insert(source.into(), library.programs.len());
        for (h, start) in headers.iter().copied().enumerate() {
            let end = headers.get(h + 1).copied().unwrap_or(blocks.len());
            let id = blocks[start].syntax["program"]
                .as_f64()
                .map(|v| v as u32)
                .or(file_id);
            let begin = if h == 0 { 0 } else { start };
            let mut labels = BTreeMap::new();
            let mut loops: Vec<(u32, usize, usize)> = Vec::new();
            for index in begin..end {
                check()?;
                if let Some(label) = blocks[index].syntax["label"].as_f64() {
                    if labels.insert(label as u32, index - begin).is_some() {
                        return Err(NcError::new(format!("Duplicate sequence label N{label}"))
                            .at(source, blocks[index].line));
                    }
                }
                let count = blocks[index].syntax["items"].as_array().map_or(0, Vec::len);
                for item_index in 0..count {
                    let item = &blocks[index].syntax["items"][item_index];
                    collect_variables(item, &mut library.variable_numbers);
                    if item["type"] == "while" {
                        let id = item["id"].as_f64().unwrap_or(0.) as u32;
                        if loops.iter().any(|l| l.0 == id) {
                            blocks[index].error = Some(NcError::new(format!(
                                "Loop DO{id} is already active in this scope."
                            )))
                        }
                        loops.push((id, index, item_index));
                    } else if item["type"] == "end" {
                        let id = item["id"].as_f64().unwrap_or(0.) as u32;
                        match loops.pop() {
                            Some((loop_id, first, slot)) if loop_id == id => {
                                blocks[first].syntax["items"][slot]["end"] =
                                    Value::from(index - begin);
                                blocks[index].syntax["items"][item_index]["start"] =
                                    Value::from(first - begin);
                            }
                            _ => {
                                blocks[index].error = Some(NcError::new(format!(
                                    "END{id} does not match an enclosing WHILE."
                                )))
                            }
                        }
                    }
                }
            }
            for (id, index, _) in loops {
                blocks[index].error =
                    Some(NcError::new(format!("WHILE DO{id} has no matching END.")))
            }
            if let Some(id) = id {
                if library.ids.insert(id, library.programs.len()).is_some() {
                    return Err(NcError::new(format!("Duplicate program number O{id}"))
                        .at(source, blocks[start].line));
                }
            }
            library.programs.push(Program {
                id,
                source: source.into(),
                blocks: Vec::new(),
                labels,
            });
        }
        let shared: Vec<_> = blocks.into_iter().map(Arc::new).collect();
        let first = *library
            .sources
            .get(source)
            .ok_or_else(|| NcError::new("Source indexing failed"))?;
        for (h, start) in headers.iter().copied().enumerate() {
            let begin = if h == 0 { 0 } else { start };
            let end = headers.get(h + 1).copied().unwrap_or(shared.len());
            library.programs[first + h].blocks = shared[begin..end].to_vec()
        }
    }
    Ok(library)
}
fn collect_variables(node: &Value, numbers: &mut BTreeSet<u32>) {
    if node["type"] == "variable" && node["address"]["type"] == "number" {
        if let Some(value) = node["address"]["value"].as_f64() {
            if (0. ..=99999.).contains(&value) && value.fract() == 0. {
                numbers.insert(value as u32);
            }
        }
    }
    match node {
        Value::Array(values) => {
            for v in values {
                collect_variables(v, numbers)
            }
        }
        Value::Object(values) => {
            for v in values.values() {
                collect_variables(v, numbers)
            }
        }
        _ => {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn libraries_labels_and_loop_links() {
        let l = compile(
            "O1000\n#100=0\nN10 WHILE[#100 LT 3]DO1\n#100=#100+1\nEND1\nM30\nO9001\nM99",
            "main.nc",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(l.programs.len(), 2);
        assert_eq!(l.programs[0].labels[&10], 2);
        assert_eq!(l.programs[0].blocks[2].syntax["items"][1]["end"], 4);
        assert!(l.variable_numbers.contains(&100));
    }
}
