//! Minimal `.xlsx` read and write for the Excel import/export tools.
//!
//! Reading uses calamine, which handles the format's quirks so callers get plain strings.
//! Writing is the small half of the spec done by hand: one sheet, inline strings, no shared
//! string table, no styles - a ZIP of six XML parts. That is enough for a data export and
//! keeps the binary free of a full spreadsheet engine. `zip` is already in the tree through
//! calamine, so no new dependency.

use calamine::{Data, Reader, Xlsx};
use nook_core::{Error, Result};
use std::io::Write;

/// Every cell of the first sheet as strings, blanks included, row-major.
pub fn read_rows<P: AsRef<std::path::Path>>(path: P) -> Result<Vec<Vec<String>>> {
    let file = std::fs::File::open(path.as_ref())
        .map_err(|e| Error::io(format!("open {}: {e}", path.as_ref().display())))?;
    let mut book: Xlsx<std::fs::File> = Xlsx::new(file)
        .map_err(|e| Error::io(format!("open xlsx: {e}")))?;
    let range = book
        .worksheet_range_at(0)
        .ok_or_else(|| Error::io("the workbook has no first sheet"))?
        .map_err(|e| Error::io(format!("read xlsx sheet: {e}")))?;
    let mut rows = Vec::new();
    for row in range.rows() {
        rows.push(
            row.iter()
                .map(|cell| match cell {
                    Data::Empty => String::new(),
                    Data::String(s) => s.clone(),
                    Data::Float(f) => format_float(*f),
                    Data::Int(i) => i.to_string(),
                    Data::Bool(b) => b.to_string(),
                    Data::DateTime(d) => d.to_string(),
                    other => other.to_string(),
                })
                .collect(),
        );
    }
    if rows.is_empty() {
        return Err(Error::io("the first sheet is empty"));
    }
    Ok(rows)
}

fn format_float(f: f64) -> String {
    // Excel keeps every number as a double; integers must not export as "1.0".
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Column letter for a 0-based index: 0 -> A, 25 -> Z, 26 -> AA.
fn column_letter(mut i: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (i % 26) as u8);
        i /= 26;
        if i == 0 {
            break;
        }
        i -= 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_else(|_| "A".to_string())
}

/// Write `rows` (header row included) as a plain one-sheet `.xlsx`.
pub fn write_rows<P: AsRef<std::path::Path>>(path: P, rows: &[Vec<String>]) -> Result<()> {
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let mut body = String::new();
    for (r, row) in rows.iter().enumerate() {
        body.push_str("<row r=\"");
        body.push_str(&(r + 1).to_string());
        body.push_str("\">");
        for (c, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            body.push_str("<c r=\"");
            body.push_str(&column_letter(c));
            body.push_str(&(r + 1).to_string());
            body.push_str("\" t=\"inlineStr\"><is><t xml:space=\"preserve\">");
            body.push_str(&xml_escape(cell));
            body.push_str("</t></is></c>");
        }
        body.push_str("</row>");
        let _ = width;
    }
    let sheet = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
         <sheetData>{body}</sheetData></worksheet>"
    );
    let content_types = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
        "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
        "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
        "<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
        "</Types>"
    );
    let root_rels = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>",
        "</Relationships>"
    );
    let workbook = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
        "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
        "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let workbook_rels = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>",
        "</Relationships>"
    );

    let file = std::fs::File::create(path.as_ref())
        .map_err(|e| Error::io(format!("create {}: {e}", path.as_ref().display())))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ] {
        zip.start_file(name, opts)
            .map_err(|e| Error::io(format!("zip {name}: {e}")))?;
        zip.write_all(bytes)
            .map_err(|e| Error::io(format!("zip {name}: {e}")))?;
    }
    zip.finish()
        .map_err(|e| Error::io(format!("finish xlsx: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_rows_read_back_unchanged() {
        let path = std::env::temp_dir().join("nook-xlsx-roundtrip.xlsx");
        write_rows(
            &path,
            &[
                vec!["id".into(), "name".into()],
                vec!["1".into(), "Ann & <Bob>".into()],
            ],
        )
            .expect("write");
        let rows = read_rows(&path).expect("read");
        assert_eq!(rows, vec![vec!["id", "name"], vec!["1", "Ann & <Bob>"]]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn blanks_read_back_as_empty_strings() {
        let path = std::env::temp_dir().join("nook-xlsx-blank.xlsx");
        write_rows(
            &path,
            &[
                vec!["a".into(), "b".into(), "c".into()],
                vec!["1".into(), String::new(), "3".into()],
            ],
        )
        .expect("write");
        let rows = read_rows(&path).expect("read");
        assert_eq!(rows, vec![vec!["a", "b", "c"], vec!["1", "", "3"]]);
        let _ = std::fs::remove_file(&path);
    }
}
