//! A flat-XML reader, without a crate.
//!
//! The shape it accepts is the one every database export uses: one element per row, one child per
//! column.
//!
//! ```xml
//! <rows>
//!   <record><id>1</id><name>Ada</name></record>
//!   <record><id>2</id><name>Grace</name></record>
//! </rows>
//! ```
//!
//! Nested XML is refused by element name rather than flattened, because "which of the three nested
//! `<value>`s is the column?" has no answer a database import can defend. Entities are decoded for
//! the five predefined names and numeric references; a DTD is refused (external entity expansion
//! is an attack, not a feature).

use std::io::BufRead;

use nook_core::{Error, Result};

/// One row: the child elements' text, in document order, plus the column names.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlRow {
    pub columns: Vec<String>,
    pub values: Vec<String>,
}

/// Rows of a parsed XML document.
///
/// Collected, not streamed: the reader pulls tokens lazily but `rows` returns every row it found,
/// so an XML import holds the whole file's rows. That is the ceiling for this path — CSV streams,
/// XML does not. Making it a real iterator means exposing the header after the first row, which is
/// a bigger change than this phase's scope.
type RowIter = Vec<Result<Vec<String>>>;

/// Read a flat XML document as a header plus rows.
///
/// The row element is inferred from the first element that has child elements, or named by the
/// caller. A nested document is refused by element name rather than silently flattened.
pub fn rows<R: BufRead>(reader: R, row_element: Option<&str>) -> Result<(Vec<String>, RowIter)> {
    let mut source = Source::new(reader);
    let mut columns: Vec<String> = Vec::new();
    let mut out: RowIter = Vec::new();
    let mut root: Option<String> = None;
    let mut row_tag: Option<String> = row_element.map(str::to_string);
    // 0 = outside the root, 1 = the root, 2 = a row, 3 = a column inside a row.
    let mut level = 0usize;
    let mut current: Option<Vec<(String, String)>> = None;
    let mut text = String::new();
    let mut open_column: Option<String> = None;

    while let Some(token) = source.next_token()? {
        match token {
            Token::Open(name, self_closing) => {
                if level == 0 && !self_closing {
                    root = Some(name);
                    level = 1;
                    continue;
                }
                // The row element is whatever sits directly under the root, unless the caller
                // named one.
                let is_row = match &row_tag {
                    Some(tag) => tag == &name,
                    None => level == 1,
                };
                if is_row && level <= 1 {
                    row_tag = Some(name);
                    level = 2;
                    current = Some(Vec::new());
                    if self_closing {
                        out.push(Ok(Vec::new()));
                        current = None;
                        level = 1;
                    }
                    continue;
                }
                if level == 2 {
                    if !self_closing {
                        level = 3;
                        open_column = Some(name);
                        text.clear();
                    } else if let Some(row) = current.as_mut() {
                        row.push((name, String::new()));
                    }
                    continue;
                }
                if level >= 3 {
                    return Err(Error::config(format!(
                        "the element <{name}> is nested inside a column of a row; nested XML is \
                         refused because a column's value would be ambiguous"
                    )));
                }
            }
            Token::Close(name) => match level {
                3 => {
                    if let (Some(row), Some(col)) = (current.as_mut(), open_column.take()) {
                        row.push((col, std::mem::take(&mut text)));
                    }
                    level = 2;
                }
                2 if row_tag.as_deref() == Some(name.as_str()) => {
                    let row = current.take().unwrap_or_default();
                    if columns.is_empty() {
                        columns = row.iter().map(|(k, _)| k.clone()).collect();
                    }
                    out.push(Ok(row.into_iter().map(|(_, v)| v).collect()));
                    level = 1;
                }
                1 => level = 0,
                _ => {}
            },
            Token::Text(chunk) => {
                if level == 3 {
                    text.push_str(&chunk);
                }
            }
        }
    }
    let _ = root;
    if columns.is_empty() {
        return Err(Error::config(
            "no rows found: expected one element per row with one child element per column",
        ));
    }
    Ok((columns, out))
}

enum Token {
    Open(String, bool),
    Close(String),
    Text(String),
}

struct Source<R: BufRead> {
    reader: R,
    buf: Vec<u8>,
    at: usize,
    done: bool,
}

impl<R: BufRead> Source<R> {
    fn new(reader: R) -> Self {
        Source {
            reader,
            buf: Vec::new(),
            at: 0,
            done: false,
        }
    }

    fn peek(&mut self) -> Result<Option<u8>> {
        self.fill(1)?;
        Ok(self.buf.get(self.at).copied())
    }

    fn fill(&mut self, want: usize) -> Result<()> {
        while self.buf.len() - self.at < want && !self.done {
            let available = self
                .reader
                .fill_buf()
                .map_err(|e| Error::io(format!("read XML: {e}")))?;
            if available.is_empty() {
                self.done = true;
                break;
            }
            self.buf.extend_from_slice(available);
            let n = available.len();
            self.reader.consume(n);
        }
        Ok(())
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        loop {
            match self.peek()? {
                None => return Ok(None),
                Some(b'<') => break,
                Some(_) => {
                    let mut text = String::new();
                    while let Some(c) = self.peek()? {
                        if c == b'<' {
                            break;
                        }
                        self.at += 1;
                        text.push(c as char);
                    }
                    if !text.trim().is_empty() {
                        return Ok(Some(Token::Text(decode_entities(&text))));
                    }
                }
            }
        }
        // `<`
        self.at += 1;
        self.fill(1)?;
        match self.buf.get(self.at).copied() {
            Some(b'?') => {
                // `<?xml ... ?>` — skip to the closing `>`.
                self.skip_past(b'>')?;
                self.next_token()
            }
            Some(b'!') => {
                self.skip_past(b'>')?;
                Err(Error::config(
                    "a DTD or comment at the top of the file is not supported; entity expansion \
                     is an attack vector, not a feature",
                ))
            }
            Some(b'/') => {
                self.at += 1;
                let name = self.read_name()?;
                self.skip_past(b'>')?;
                Ok(Some(Token::Close(name)))
            }
            _ => {
                let name = self.read_name()?;
                let mut self_closing = false;
                loop {
                    self.fill(1)?;
                    match self.buf.get(self.at).copied() {
                        None => break,
                        Some(b'>') => {
                            self.at += 1;
                            break;
                        }
                        Some(b'/') => {
                            self.fill(2)?;
                            if self.buf.get(self.at + 1) == Some(&b'>') {
                                self.at += 2;
                                self_closing = true;
                                break;
                            }
                            self.at += 1;
                        }
                        Some(_) => {
                            // An attribute: skip it whole, quotes and all.
                            self.skip_attribute()?;
                        }
                    }
                }
                Ok(Some(Token::Open(name, self_closing)))
            }
        }
    }

    /// Skip `name="value"` up to and including the closing quote, or a bare token.
    fn skip_attribute(&mut self) -> Result<()> {
        while let Some(c) = self.peek()? {
            if c == b'=' {
                self.at += 1;
                self.fill(1)?;
                if let Some(q @ (b'"' | b'\'')) = self.buf.get(self.at).copied() {
                    self.at += 1;
                    self.skip_past(q)?;
                }
                return Ok(());
            }
            if c == b'>' || c == b'/' {
                return Ok(());
            }
            self.at += 1;
        }
        Ok(())
    }

    fn skip_past(&mut self, byte: u8) -> Result<()> {
        while let Some(c) = self.peek()? {
            self.at += 1;
            if c == byte {
                return Ok(());
            }
        }
        Err(Error::config("the XML ended inside a tag"))
    }

    fn read_name(&mut self) -> Result<String> {
        let mut name = String::new();
        while let Some(c) = self.peek()? {
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':' | b'.') {
                self.at += 1;
                name.push(c as char);
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Err(Error::config("a tag with no name"));
        }
        Ok(name)
    }
}

/// The five predefined entities plus numeric references.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.trim().to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                // An unknown entity stays literal rather than vanishing.
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
        let (columns, rows) = rows(text.as_bytes(), None)?;
        let rows = rows.into_iter().collect::<Result<Vec<_>>>()?;
        Ok((columns, rows))
    }

    #[test]
    fn a_flat_document_becomes_a_header_and_rows() {
        let xml = "<?xml version=\"1.0\"?>\n<rows>\n\
                   <record><id>1</id><name>Ada</name></record>\n\
                   <record><id>2</id><name>Grace</name></record>\n</rows>";
        let (columns, rows) = parse(xml).expect("parses");
        assert_eq!(columns, vec!["id", "name"]);
        assert_eq!(rows, vec![vec!["1", "Ada"], vec!["2", "Grace"]]);
    }

    #[test]
    fn entities_and_cdata_free_text_are_decoded() {
        let xml = "<r><record><v>a &amp; b</v><w>&#65;</w></record></r>";
        let (_, rows) = parse(xml).expect("parses");
        assert_eq!(rows, vec![vec!["a & b", "A"]]);
    }

    #[test]
    fn nesting_is_refused_by_name() {
        let xml = "<r><record><v><inner>x</inner></v></record></r>";
        let err = parse(xml).expect_err("nested");
        assert!(err.message().contains("inner"), "{}", err.message());
    }

    #[test]
    fn a_named_row_element_is_honoured() {
        let xml = "<data><item><a>1</a></item><item><a>2</a></item></data>";
        let (columns, rows) = rows(xml.as_bytes(), Some("item")).expect("parses");
        assert_eq!(columns, vec!["a"]);
        let rows: Vec<Vec<String>> = rows.into_iter().collect::<Result<_>>().expect("rows");
        assert_eq!(rows, vec![vec!["1"], vec!["2"]]);
    }
}
