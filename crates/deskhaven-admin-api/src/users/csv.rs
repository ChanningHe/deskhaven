//! A small RFC 4180 reader for the account import: comma separated, fields optionally in
//! double quotes (`""` is a literal quote; quoted fields may hold commas and line breaks),
//! records end at LF or CRLF, a leading UTF-8 BOM is ignored, blank lines are skipped.
//! Errors name a line, never a field's content.

/// One record and the (1-based) line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub line: usize,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CsvError {
    #[error("line {line}: a quoted field is not closed")]
    Unclosed { line: usize },
    #[error("line {line}: unexpected character after a closing quote")]
    AfterClosing { line: usize },
    #[error("line {line}: a quote inside an unquoted field")]
    Stray { line: usize },
}

#[derive(Default)]
struct Reader {
    records: Vec<Record>,
    fields: Vec<String>,
    field: String,
    line: usize,
    start: usize,
    /// The current field started with a quote (and may have ended it).
    quoted: bool,
    in_quotes: bool,
    /// The current record has any content (a quote counts, even around nothing).
    any: bool,
}

impl Reader {
    fn end_field(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
        self.quoted = false;
    }

    fn end_record(&mut self) {
        self.end_field();
        let fields = std::mem::take(&mut self.fields);
        if self.any {
            self.records.push(Record {
                line: self.start,
                fields,
            });
        }
        self.any = false;
        self.start = self.line + 1;
    }

    fn unquoted(&mut self, c: char) -> Result<(), CsvError> {
        let line = self.line;
        match c {
            ',' => {
                self.any = true;
                self.end_field();
            }
            '\n' => self.end_record(),
            '"' if self.field.is_empty() && !self.quoted => {
                self.quoted = true;
                self.in_quotes = true;
                self.any = true;
            }
            '"' if self.quoted => return Err(CsvError::AfterClosing { line }),
            '"' => return Err(CsvError::Stray { line }),
            _ if self.quoted => return Err(CsvError::AfterClosing { line }),
            c => {
                self.any = true;
                self.field.push(c);
            }
        }
        Ok(())
    }
}

/// Every non-blank record of `text`.
pub(crate) fn parse(text: &str) -> Result<Vec<Record>, CsvError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut r = Reader {
        line: 1,
        start: 1,
        ..Reader::default()
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        // CRLF -> LF; a lone CR is kept as data.
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        if r.in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    r.field.push('"');
                }
                '"' => r.in_quotes = false,
                c => r.field.push(c),
            }
        } else {
            r.unquoted(c)?;
        }
        if c == '\n' {
            r.line += 1;
        }
    }
    if r.in_quotes {
        return Err(CsvError::Unclosed { line: r.start });
    }
    r.end_record();
    Ok(r.records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(line: usize, fields: &[&str]) -> Record {
        Record {
            line,
            fields: fields.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    #[test]
    fn plain_and_quoted_fields() {
        let text = "\u{feff}username,note\r\nalice,\"a, \"\"quoted\"\" note\"\nbob,\n\n";
        assert_eq!(
            parse(text).unwrap(),
            [
                rec(1, &["username", "note"]),
                rec(2, &["alice", "a, \"quoted\" note"]),
                rec(3, &["bob", ""]),
            ]
        );
    }

    #[test]
    fn quoted_line_breaks_keep_the_starting_line() {
        let text = "a,b\n\"multi\nline\",x\nlast,y";
        assert_eq!(
            parse(text).unwrap(),
            [
                rec(1, &["a", "b"]),
                rec(2, &["multi\nline", "x"]),
                rec(4, &["last", "y"]),
            ]
        );
    }

    #[test]
    fn malformed_quotes_name_the_line() {
        assert_eq!(parse("a\n\"open"), Err(CsvError::Unclosed { line: 2 }));
        assert_eq!(parse("a\nx\"y"), Err(CsvError::Stray { line: 2 }));
        assert_eq!(parse("\"a\"b"), Err(CsvError::AfterClosing { line: 1 }));
    }

    #[test]
    fn an_empty_quoted_field_is_a_record() {
        assert_eq!(parse("\"\"\n").unwrap(), [rec(1, &[""])]);
        assert!(parse("").unwrap().is_empty());
    }
}
