//! Module-owned RFC 4180 reader for opening items (same format rules as
//! Reconciliation; no dependency on that sibling): one configurable delimiter,
//! double-quoted fields with doubled quotes inside them, fields that span
//! lines, CRLF, LF or CR line ends, and a UTF-8 byte-order mark. Nothing here
//! is specific to an account.

/// The largest file the reader accepts, in bytes.
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Why a file is not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CsvError {
    TooLarge,
    NotUtf8,
    /// A quote in the middle of an unquoted field, or text after a closing
    /// quote; the row (a record, however many lines it spans) holds it.
    StrayQuote(usize),
    /// A field quote left open; the row is the one that opened it.
    UnterminatedQuote(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Outside quotes.
    Field,
    Quoted,
    /// Just after a closing quote, where only a delimiter or a line end may follow.
    Closed,
}

/// The file's rows as lists of strings. Blank rows are dropped.
pub(crate) fn parse(content: &[u8], delimiter: &str) -> Result<Vec<Vec<String>>, CsvError> {
    if content.len() > MAX_BYTES {
        return Err(CsvError::TooLarge);
    }
    let text = std::str::from_utf8(content).map_err(|_| CsvError::NotUtf8)?;
    let mut rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut row_number = 1;
    let mut state = State::Field;
    while !rest.is_empty() {
        if rest.starts_with('"') {
            match state {
                State::Field if field.is_empty() => state = State::Quoted,
                State::Quoted if rest.starts_with("\"\"") => {
                    field.push('"');
                    rest = &rest[1..];
                }
                State::Quoted => state = State::Closed,
                _ => return Err(CsvError::StrayQuote(row_number)),
            }
            rest = &rest[1..];
            continue;
        }
        if state != State::Quoted {
            let line_end = ["\r\n", "\n", "\r"]
                .into_iter()
                .find(|end| rest.starts_with(end));
            if let Some(end) = line_end {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                row_number += 1;
                state = State::Field;
                rest = &rest[end.len()..];
                continue;
            }
            if let Some(tail) = rest.strip_prefix(delimiter) {
                row.push(std::mem::take(&mut field));
                state = State::Field;
                rest = tail;
                continue;
            }
            if state == State::Closed {
                return Err(CsvError::StrayQuote(row_number));
            }
            state = State::Field;
        }
        let character = rest.chars().next().expect("rest is not empty");
        field.push(character);
        rest = &rest[character.len_utf8()..];
    }
    if state == State::Quoted {
        return Err(CsvError::UnterminatedQuote(row_number));
    }
    if !(field.is_empty() && row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|row| !row.iter().all(|cell| cell.trim().is_empty()));
    Ok(rows)
}
