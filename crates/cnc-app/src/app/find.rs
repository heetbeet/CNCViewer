use std::ops::Range;

use regex::{Regex, RegexBuilder};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Find {
    pub open: bool,
    pub replace_open: bool,
    pub query: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub reveal: bool,
    #[serde(skip)]
    pub results: Option<Results>,
}

#[derive(Clone)]
pub(super) struct Results {
    pub source: String,
    pub ranges: Vec<Range<usize>>,
    pub line_starts: Vec<usize>,
    pub active: usize,
    pub error: Option<String>,
    pattern: Option<Regex>,
}

impl Find {
    pub fn refresh(&mut self, source: &str, text: &str, anchor: usize) {
        let mut results = Results {
            source: source.to_owned(),
            ranges: Vec::new(),
            line_starts: Vec::new(),
            active: 0,
            error: None,
            pattern: None,
        };
        if !self.query.is_empty() {
            let pattern = if self.regex {
                self.query.clone()
            } else {
                regex::escape(&self.query)
            };
            match RegexBuilder::new(&pattern)
                .case_insensitive(!self.case_sensitive)
                .multi_line(true)
                .build()
            {
                Ok(pattern) => {
                    results.ranges.extend(
                        pattern
                            .find_iter(text)
                            .map(|matched| matched.range())
                            .filter(|range| !self.whole_word || at_word_boundaries(text, range)),
                    );
                    if !results.ranges.is_empty() {
                        let index = results.ranges.partition_point(|range| range.start < anchor);
                        results.active = index % results.ranges.len();
                        results.line_starts.push(0);
                        results
                            .line_starts
                            .extend(text.match_indices('\n').map(|(byte, _)| byte + 1));
                    }
                    results.pattern = Some(pattern);
                }
                Err(error) => results.error = Some(error.to_string()),
            }
        }
        self.results = Some(results);
    }

    pub fn advance(&mut self, backwards: bool) {
        if let Some(results) = self.results.as_mut().filter(|r| !r.ranges.is_empty()) {
            let count = results.ranges.len();
            results.active = if backwards {
                results.active.checked_sub(1).unwrap_or(count - 1)
            } else {
                (results.active + 1) % count
            };
            self.reveal = true;
        }
    }

    pub fn replacement_text(&self, text: &str, all: bool) -> Option<(String, usize)> {
        let results = self.results.as_ref()?;
        let active = results.ranges.get(results.active)?;
        let ranges = if all {
            &results.ranges
        } else {
            std::slice::from_ref(active)
        };
        let mut replaced = String::with_capacity(text.len());
        let mut end = 0;
        let mut next = 0;
        for range in ranges {
            replaced.push_str(&text[end..range.start]);
            if all && range == active {
                next = replaced.len();
            }
            if self.regex {
                let pattern = results.pattern.as_ref()?;
                let captures = pattern.captures_at(text, range.start)?;
                captures.expand(&self.replacement, &mut replaced);
            } else {
                replaced.push_str(&self.replacement);
            }
            end = range.end;
        }
        if !all {
            next = replaced.len();
        }
        replaced.push_str(&text[end..]);
        Some((replaced, next))
    }
}

impl Results {
    pub fn current(&self) -> Option<&Range<usize>> {
        self.ranges.get(self.active)
    }

    pub fn row_at(&self, byte: usize) -> usize {
        self.line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }
}

fn at_word_boundaries(text: &str, range: &Range<usize>) -> bool {
    let word = |character: char| character.is_alphanumeric() || character == '_';
    !text[..range.start].chars().next_back().is_some_and(word)
        && !text[range.end..].chars().next().is_some_and(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_search_uses_unicode_case_and_whole_words() {
        let text = "(ÉTAGE étage Étages)\nX#100 #100 #1000";
        let mut find = Find {
            query: "étage".into(),
            whole_word: true,
            ..Find::default()
        };
        find.refresh("program.nc", text, 3);
        let results = find.results.as_ref().unwrap();
        assert_eq!(results.ranges.len(), 2);
        assert_eq!(&text[results.current().unwrap().clone()], "étage");
        find.advance(false);
        assert_eq!(find.results.as_ref().unwrap().active, 0);
        find.advance(true);
        assert_eq!(find.results.as_ref().unwrap().active, 1);

        find.query = "#100".into();
        find.refresh("program.nc", text, 0);
        let results = find.results.as_ref().unwrap();
        assert_eq!(results.ranges.len(), 1);
        assert_eq!(results.row_at(results.current().unwrap().start), 1);
    }

    #[test]
    fn regex_replacement_handles_captures_and_updates_the_offset() {
        let mut find = Find {
            query: "X(\\d+)".into(),
            replacement: "Y$1".into(),
            case_sensitive: true,
            regex: true,
            ..Find::default()
        };
        let text = "X120\nx12\nX8";
        find.refresh("program.nc", text, 5);
        assert_eq!(
            find.replacement_text(text, false),
            Some(("X120\nx12\nY8".into(), 11))
        );
        assert_eq!(
            find.replacement_text(text, true),
            Some(("Y120\nx12\nY8".into(), 9))
        );
        find.replacement = "Y".into();
        assert_eq!(
            find.replacement_text(text, true),
            Some(("Y\nx12\nY".into(), 6))
        );

        find.query = "^".into();
        find.replacement = "N1 ".into();
        find.refresh("program.nc", text, 0);
        assert_eq!(
            find.replacement_text(text, true).unwrap().0,
            "N1 X120\nN1 x12\nN1 X8"
        );

        find.query = "[".into();
        find.refresh("program.nc", text, 0);
        assert!(find.results.as_ref().unwrap().error.is_some());
        assert!(find.replacement_text(text, true).is_none());
    }
}
