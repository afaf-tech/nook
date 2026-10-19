//! CSV: one parser, driven by a reader.
//!
//! The import path used to `read_to_string` the whole file and parse a `String`. That is the defect
//! this module removes: a 500 MB file was 500 MB of RAM plus a second copy while the rows were
//! reordered. The state machine is the same one — it just pulls characters from a `BufRead` and
//! yields a row as soon as its newline arrives, so peak memory is one row plus the buffer.
//!
//! `parse_csv` is kept as a wrapper over `rows`, so there is exactly one parser and the existing
//! tests still pin the behaviour.

use std::io::{BufRead, Read};

use nook_core::{Error, Result};

/// Rows from a CSV stream. Quoted fields may hold the delimiter, doubled quotes and newlines.
///
/// The reader is deliberately `char`-oriented rather than byte-oriented: a delimiter can be any
/// character, and a UTF-8 continuation byte must never be mistaken for one. `Chars` over a
/// `BufReader` keeps the buffer, so the cost is one `char` decode per byte rather than one `read`
/// per byte.
pub struct Rows<R: BufRead> {
    source: CharSource<R>,
    delimiter: char,
    /// Set once the stream ends or a read fails, so the iterator is fused.
    done: bool,
    /// Rows are yielded with the line they started on, for a failure report a human can act on.
    line: usize,
}

impl<R: BufRead> Rows<R> {
    /// Bytes the reader is currently holding. Only for tests: it exists so the bounded-memory
    /// claim is a measured assertion instead of a comment.
    #[cfg(test)]
    fn buffered_bytes(&self) -> usize {
        self.source.buf.len()
    }

    pub fn new(reader: R, delimiter: char) -> Self {
        Rows {
            source: CharSource::new(reader),
            delimiter,
            done: false,
            line: 1,
        }
    }
}

/// Decode UTF-8 one character at a time over a `BufRead`, without copying the file.
///
/// A character split across two `fill_buf` calls is the case that matters: at a 64 KB boundary a
/// multi-byte character is cut in half roughly once per 8,000 bytes, so a reader that assumed a
/// character never straddles a buffer would fail on any non-ASCII file. The tail of the buffer is
/// kept and the next read is appended to it.
struct CharSource<R: BufRead> {
    reader: R,
    buf: Vec<u8>,
    at: usize,
}

impl<R: BufRead> CharSource<R> {
    fn new(reader: R) -> Self {
        CharSource {
            reader,
            buf: Vec::new(),
            at: 0,
        }
    }

    fn next_char(&mut self) -> std::io::Result<Option<char>> {
        loop {
            if self.at < self.buf.len() {
                match decode_one(&self.buf[self.at..]) {
                    Ok(Some((c, used))) => {
                        self.at += used;
                        return Ok(Some(c));
                    }
                    // A partial character at the end of what we hold: keep those bytes and read
                    // more rather than consuming them.
                    Ok(None) => self.compact(),
                    Err(()) => return Err(invalid_utf8(&self.buf[self.at..])),
                }
            }
            // Every byte decoded so far is dead weight. Dropping them here — not only in the
            // partial-character branch — is what keeps the buffer at one chunk instead of the
            // whole file: without it a 543 MB CSV leaves 543 MB resident.
            self.compact();
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                return if self.at < self.buf.len() {
                    Err(invalid_utf8(&self.buf[self.at..]))
                } else {
                    Ok(None)
                };
            }
            let n = available.len();
            self.buf.extend_from_slice(available);
            self.reader.consume(n);
        }
    }

    /// Drop the bytes already decoded, so the buffer stays one chunk plus a partial character.
    fn compact(&mut self) {
        if self.at > 0 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
    }
}

fn invalid_utf8(bytes: &[u8]) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "invalid UTF-8 starting at byte {:#04x}",
            bytes.first().unwrap_or(&0)
        ),
    )
}

/// Decode the character at the front of `bytes`. `Ok(None)` means more bytes are needed.
fn decode_one(bytes: &[u8]) -> std::result::Result<Option<(char, usize)>, ()> {
    let Some(&first) = bytes.first() else {
        return Ok(None);
    };
    let len = match first {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return Err(()),
    };
    if bytes.len() < len {
        return Ok(None);
    }
    match std::str::from_utf8(&bytes[..len]) {
        Ok(s) => Ok(s.chars().next().map(|c| (c, len))),
        Err(_) => Err(()),
    }
}

impl<R: BufRead> Iterator for Rows<R> {
    type Item = Result<Vec<String>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut row: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;
        let mut quoted_field = false;
        let mut started = false;

        loop {
            let next = match self.source.next_char() {
                Ok(Some(c)) => Some(c),
                Ok(None) => None,
                Err(e) => {
                    self.done = true;
                    return Some(Err(Error::io(format!(
                        "reading the file as UTF-8 failed at line {}: {e}. Pass \"encoding\": \"latin1\" \
                         for a file in a legacy 8-bit encoding",
                        self.line
                    ))));
                }
            };
            let Some(c) = next else {
                self.done = true;
                if !started {
                    return None;
                }
                row.push(std::mem::take(&mut field));
                return Some(Ok(row));
            };
            started = true;

            if in_quotes {
                if c == '"' {
                    match self.source.next_char() {
                        Ok(Some('"')) => field.push('"'),
                        Ok(Some(other)) => {
                            in_quotes = false;
                            if other == self.delimiter {
                                row.push(std::mem::take(&mut field));
                                quoted_field = false;
                            } else if other == '\n' {
                                self.line += 1;
                                row.push(std::mem::take(&mut field));
                                return Some(Ok(row));
                            } else if other != '\r' {
                                // Text after the closing quote is kept, as RFC 4180's readers do.
                                field.push(other);
                            }
                        }
                        Ok(None) => {
                            self.done = true;
                            row.push(std::mem::take(&mut field));
                            return Some(Ok(row));
                        }
                        Err(e) => {
                            self.done = true;
                            return Some(Err(Error::io(format!(
                                "reading the file as UTF-8 failed at line {}: {e}",
                                self.line
                            ))));
                        }
                    }
                } else {
                    if c == '\n' {
                        self.line += 1;
                    }
                    field.push(c);
                }
                continue;
            }

            match c {
                '"' if field.is_empty() && !quoted_field => {
                    in_quotes = true;
                    quoted_field = true;
                }
                c if c == self.delimiter => {
                    row.push(std::mem::take(&mut field));
                    quoted_field = false;
                }
                '\r' => {}
                '\n' => {
                    self.line += 1;
                    row.push(std::mem::take(&mut field));
                    // A blank line is not a row: trailing newlines and stray CRLFs are common.
                    if row.len() == 1 && row[0].trim().is_empty() {
                        row.clear();
                        started = false;
                        continue;
                    }
                    return Some(Ok(row));
                }
                _ => field.push(c),
            }
        }
    }
}

/// Rows from any `BufRead`.
pub fn rows<R: BufRead>(reader: R, delimiter: char) -> Rows<R> {
    Rows::new(reader, delimiter)
}

/// The whole-file parse, kept for callers that already hold the text. Same parser, so a test that
/// pins one pins both.
pub fn parse_csv(text: &str, delimiter: char) -> Vec<Vec<String>> {
    rows(text.as_bytes(), delimiter)
        .filter_map(std::result::Result::ok)
        .collect()
}

/// Strip a UTF-8 BOM if present, without copying the body.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

/// Open a file for streaming, decoding `latin1` when asked. Returns a boxed reader so the caller
/// does not grow a type parameter for the encoding choice.
pub fn open(path: &std::path::Path, latin1: bool) -> Result<Box<dyn BufRead>> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::io(format!("open {}: {e}", path.display())))?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
    if latin1 {
        return Ok(Box::new(std::io::BufReader::with_capacity(
            64 * 1024,
            Latin1::new(reader),
        )));
    }
    let head = reader
        .fill_buf()
        .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        reader.consume(3);
    }
    Ok(Box::new(reader))
}

/// Latin-1 is a byte-to-codepoint map, so it decodes one byte at a time without ever holding the
/// file. Bytes are widened into UTF-8 on the way out; the `BufReader` around it absorbs the small
/// writes.
struct Latin1<R: BufRead> {
    reader: R,
    out: Vec<u8>,
    at: usize,
}

impl<R: BufRead> Latin1<R> {
    fn new(reader: R) -> Self {
        Latin1 {
            reader,
            out: Vec::new(),
            at: 0,
        }
    }
}

impl<R: BufRead> Read for Latin1<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.at >= self.out.len() {
            let input = self.reader.fill_buf()?;
            if input.is_empty() {
                return Ok(0);
            }
            let taken = input.len();
            self.out.clear();
            self.out.reserve(taken * 2);
            for byte in input {
                if *byte < 0x80 {
                    self.out.push(*byte);
                } else {
                    self.out.push(0xC0 | (*byte >> 6));
                    self.out.push(0x80 | (*byte & 0x3F));
                }
            }
            self.reader.consume(taken);
            self.at = 0;
        }
        let n = (self.out.len() - self.at).min(buf.len());
        buf[..n].copy_from_slice(&self.out[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<Vec<String>> {
        parse_csv(text, ',')
    }

    #[test]
    fn quotes_commas_and_newlines_survive() {
        let rows = parse("a,b\n\"x,y\",\"line\nbreak\"\n\"he said \"\"hi\"\"\",2\n");
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["x,y".to_string(), "line\nbreak".to_string()],
                vec!["he said \"hi\"".to_string(), "2".to_string()],
            ]
        );
    }

    #[test]
    fn crlf_and_a_trailing_newline_do_not_add_a_row() {
        assert_eq!(
            parse("a,b\r\n1,2\r\n"),
            vec![vec!["a", "b"], vec!["1", "2"]]
        );
    }

    #[test]
    fn a_bom_is_stripped_before_the_header() {
        let bytes = [0xEF, 0xBB, 0xBF, b'a', b',', b'b', b'\n'];
        let parsed: Vec<Vec<String>> = rows(strip_bom(&bytes), ',')
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(parsed, vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn an_empty_field_at_the_end_of_a_row_is_kept() {
        assert_eq!(parse("a,b,\n"), vec![vec!["a", "b", ""]]);
    }

    #[test]
    fn a_chunked_read_yields_the_same_rows_as_the_whole_file() {
        // A reader that hands out one byte at a time is the worst case for a parser that assumes
        // a row arrives in one buffer.
        // Non-ASCII on purpose: a character split across a read boundary is the case a purely
        // ASCII fixture cannot catch.
        let text = "id,name\n1,\"a,b\"\n2,\"c\nd\"\n3,café\n4,日本\n";
        let whole = parse_csv(text, ',');
        let chunked: Vec<Vec<String>> = rows(OneByte(text.as_bytes()), ',')
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(whole, chunked);
    }

    #[test]
    fn the_reader_does_not_hold_the_bytes_it_has_already_decoded() {
        // The regression this pins: `compact` used to run only on the partial-character branch, so
        // on an all-ASCII file the buffer grew to the whole file — a 543 MB CSV held 543 MB
        // resident. The buffer must stay bounded by one read chunk.
        let mut text = String::from("id,name\n");
        for i in 0..200_000 {
            text.push_str(&format!("{i},name-{i}\n"));
        }
        // `Cursor::fill_buf` hands out the whole remainder, so the buffer has to be capped the way
        // `csv::open` caps it or this measures the fixture instead of the reader.
        let capped =
            std::io::BufReader::with_capacity(64 * 1024, std::io::Cursor::new(text.into_bytes()));
        let mut reader = rows(capped, ',');
        let mut peak = 0usize;
        for _ in 0..150_000 {
            if reader.next().is_none() {
                break;
            }
            peak = peak.max(reader.buffered_bytes());
        }
        assert!(
            peak < 64 * 1024 * 2,
            "the reader held {peak} bytes; it is accumulating the file instead of streaming it"
        );
    }

    #[test]
    fn invalid_utf8_is_refused_with_the_line() {
        let bad: &[u8] = b"a,b\n1,\xff\n";
        let err = rows(bad, ',').find_map(|r| r.err()).expect("refused");
        assert!(err.message().contains("UTF-8"), "{}", err.message());
        assert!(err.message().contains("latin1"), "{}", err.message());
    }

    /// A `BufRead` that returns at most one byte per `fill_buf`, to prove the parser never assumes
    /// a whole row is buffered.
    struct OneByte<'a>(&'a [u8]);

    impl Read for OneByte<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() || out.is_empty() {
                return Ok(0);
            }
            out[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    impl BufRead for OneByte<'_> {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Ok(self.0.get(..1).unwrap_or(&[]))
        }
        fn consume(&mut self, amount: usize) {
            self.0 = &self.0[amount.min(self.0.len())..];
        }
    }
}
