// Portions adapted from asukaminato0721/krkrz_rs
// b4a295fabbedf9ff5579faf304b0342d7a0699a6 (crates/krkrz_runtime/src/csv.rs),
// MIT License (docs/third-party/krkrz_rs.txt). Modifications CrySirror
// contributors (MPL-2.0).
//! Line-oriented CSV reader with the KiriKiri CSVParser rules: one separator
//! unit, `"` quoting with `""` escapes, quoted fields spanning lines (joined
//! with the configured newline), and byte input in UTF-8 or CP932. Text after
//! a closing quote up to the separator is kept. No VM is involved.
use anyhow::{bail, Result};

/// Upper bound for byte/text input and for one multi-line field.
pub const MAX_INPUT: usize = 64 << 20;

enum Input {
    Text(Vec<u16>),
    Bytes { data: Vec<u8>, utf8: bool },
}

pub struct CsvReader {
    pub separator: u16,
    pub newline: Vec<u16>,
    input: Option<Input>,
    pos: usize,
    pub line_number: i64,
}

impl Default for CsvReader {
    fn default() -> Self {
        Self {
            separator: u16::from(b','),
            newline: vec![13, 10],
            input: None,
            pos: 0,
            line_number: 0,
        }
    }
}

impl CsvReader {
    pub fn init_text(&mut self, input: Vec<u16>) -> Result<()> {
        if input.len() > MAX_INPUT {
            bail!("CSV input exceeds limit");
        }
        self.input = Some(Input::Text(input));
        self.pos = 0;
        self.line_number = 0;
        Ok(())
    }
    pub fn init_bytes(&mut self, data: Vec<u8>, utf8: bool) -> Result<()> {
        if data.len() > MAX_INPUT {
            bail!("CSV input exceeds limit");
        }
        self.input = Some(Input::Bytes { data, utf8 });
        self.pos = 0;
        self.line_number = 0;
        Ok(())
    }
    pub fn close(&mut self) {
        self.input = None;
    }
    fn add_line(&mut self, line: &mut Vec<u16>) -> Result<bool> {
        let Some(input) = &self.input else {
            return Ok(false);
        };
        match input {
            Input::Bytes { data, utf8 } => {
                let start = self.pos;
                while self.pos < data.len() && ![10, 13, 255].contains(&data[self.pos]) {
                    self.pos += 1;
                }
                let end = self.pos;
                let terminator = data.get(self.pos).copied();
                if terminator.is_some() {
                    self.pos += 1;
                }
                // The native reader's buffered CR/LF handling, including its
                // 8 KiB block-boundary quirk at the very end of the input.
                if terminator == Some(13)
                    && self.pos < data.len()
                    && (data[self.pos] == 10
                        || self.pos + 1 == data.len() && !data.len().is_multiple_of(8192))
                {
                    self.pos += 1;
                }
                if end == start && (terminator.is_none() || terminator == Some(255)) {
                    return Ok(false);
                }
                let text: String = if *utf8 {
                    std::str::from_utf8(&data[start..end])?.into()
                } else {
                    let (text, errors) =
                        encoding_rs::SHIFT_JIS.decode_without_bom_handling(&data[start..end]);
                    if errors {
                        bail!("CSV malformed CP932 replacement rules are not implemented");
                    }
                    text.into_owned()
                };
                // Decoded text is appended as a NUL-terminated string.
                line.extend(text.split('\0').next().unwrap_or("").encode_utf16());
                Ok(true)
            }
            Input::Text(input) => {
                if self.pos == input.len() {
                    return Ok(false);
                }
                while let Some(&unit) = input.get(self.pos) {
                    self.pos += 1;
                    if unit == 13 {
                        // The string reader drops a final non-LF unit after CR.
                        if self.pos < input.len()
                            && (input[self.pos] == 10 || self.pos + 1 == input.len())
                        {
                            self.pos += 1;
                        }
                        break;
                    }
                    if unit == 10 {
                        break;
                    }
                    line.push(unit);
                }
                Ok(true)
            }
        }
    }
    /// Next record, or `None` at the end (which also releases the input).
    pub fn next_record(&mut self) -> Result<Option<Vec<Vec<u16>>>> {
        let mut line = vec![];
        if !self.add_line(&mut line)? {
            self.input = None;
            return Ok(None);
        }
        self.line_number += 1;
        let mut fields = vec![];
        if line.is_empty() {
            return Ok(Some(fields));
        }
        let mut i = 0;
        loop {
            let mut field = vec![];
            if line.get(i) == Some(&34) {
                i += 1;
                'quoted: loop {
                    while i < line.len() {
                        let unit = line[i];
                        i += 1;
                        if unit == 34 {
                            if line.get(i) != Some(&34) {
                                while i < line.len() && line[i] != self.separator {
                                    field.push(line[i]);
                                    i += 1;
                                }
                                break 'quoted;
                            }
                            i += 1;
                        }
                        field.push(unit);
                    }
                    if field.len().saturating_add(self.newline.len()) > MAX_INPUT / 2 {
                        bail!("CSV field exceeds limit");
                    }
                    field.extend_from_slice(&self.newline);
                    if !self.add_line(&mut line)? {
                        break;
                    }
                }
            } else {
                while i < line.len() && line[i] != self.separator {
                    field.push(line[i]);
                    i += 1;
                }
            }
            fields.push(field);
            if i >= line.len() {
                break;
            }
            i += 1;
        }
        Ok(Some(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[u16]) -> String {
        String::from_utf16(v).unwrap()
    }
    #[test]
    fn quoted_multiline_and_cp932() {
        let mut r = CsvReader::default();
        r.init_text("a,\"b,\"\"c\"\"\",d\r\n\"x\ny\",z\n".encode_utf16().collect()).unwrap();
        let row = r.next_record().unwrap().unwrap();
        assert_eq!(row.iter().map(|f| s(f)).collect::<Vec<_>>(), ["a", "b,\"c\"", "d"]);
        let row = r.next_record().unwrap().unwrap();
        assert_eq!(s(&row[0]), "x\r\ny");
        assert_eq!(r.line_number, 2);
        assert!(r.next_record().unwrap().is_none());
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("名前,値\n");
        r.init_bytes(sjis.into_owned(), false).unwrap();
        assert_eq!(s(&r.next_record().unwrap().unwrap()[0]), "名前");
    }
}
