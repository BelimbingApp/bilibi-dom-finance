//! A bank account's import mapping (Bilimbi
//! `Bilimbi.Finance.Reconciliation.Mapping`): which columns of its bank's CSV
//! export hold a line's date, description, reference, amount (or debit and
//! credit) and running balance, and how the bank writes dates and amounts.
//!
//! The mapping is stored with the bank account as JSON, so each bank's export
//! is a company's configured data, never code. A column is named by its
//! header text, or by its 1-based position when the file has no header row.
//!
//! | Key | Values | Default |
//! | --- | --- | --- |
//! | `delimiter` | `","`, `";"`, `"\t"`, `"|"` | `","` |
//! | `header_rows` | 0 to 20 rows before the data; the last is the header | `1` |
//! | `date` | column | required |
//! | `date_format` | one of [`DATE_FORMATS`] | `"YYYY-MM-DD"` |
//! | `description` | column | required |
//! | `reference` | column or null | null |
//! | `amount` | column: one signed column | either this or `debit` and `credit` |
//! | `amount_sign` | `"deposits_positive"` or `"withdrawals_positive"` | `"deposits_positive"` |
//! | `debit` | column: money out | with `credit` |
//! | `credit` | column: money in | with `debit` |
//! | `balance` | column or null: the running balance | null |
//! | `decimal_separator` | `"."` or `","` | `"."` |
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The column separators a bank's export may use.
pub const DELIMITERS: [&str; 4] = [",", ";", "\t", "|"];
/// The nine date forms a bank's export may use.
pub const DATE_FORMATS: [&str; 9] = [
    "YYYY-MM-DD",
    "DD/MM/YYYY",
    "MM/DD/YYYY",
    "DD-MM-YYYY",
    "DD.MM.YYYY",
    "YYYY/MM/DD",
    "YYYYMMDD",
    "DD-MMM-YYYY",
    "DD-MMM-YY",
];
/// Which sign of a single amount column means money in.
pub const AMOUNT_SIGNS: [&str; 2] = ["deposits_positive", "withdrawals_positive"];
/// The decimal point of the bank's amounts.
pub const DECIMAL_SEPARATORS: [&str; 2] = [".", ","];

const COLUMNS: [&str; 7] = [
    "date",
    "description",
    "reference",
    "amount",
    "debit",
    "credit",
    "balance",
];
const KEYS: [&str; 12] = [
    "delimiter",
    "header_rows",
    "date_format",
    "amount_sign",
    "decimal_separator",
    "date",
    "description",
    "reference",
    "amount",
    "debit",
    "credit",
    "balance",
];
const MAX_HEADER_ROWS: i64 = 20;
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// A column of the file: its header text, or its 1-based position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Column {
    Position(u32),
    Header(String),
}

/// A bank account's import mapping with every key present and its defaults
/// filled. Built by [`ImportMapping::parse`] from the map a company enters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMapping {
    pub delimiter: String,
    pub header_rows: u8,
    pub date: Option<Column>,
    pub date_format: String,
    pub description: Option<Column>,
    pub reference: Option<Column>,
    pub amount: Option<Column>,
    pub amount_sign: String,
    pub debit: Option<Column>,
    pub credit: Option<Column>,
    pub balance: Option<Column>,
    pub decimal_separator: String,
}

impl ImportMapping {
    /// A mapping with every key present and its defaults filled, or why it is
    /// invalid (the sentence a form shows beside `import_mapping`).
    pub fn parse(mapping: &Value) -> Result<Self, String> {
        let Value::Object(map) = mapping else {
            return Err("must be a map of import settings".into());
        };
        let mut unknown: Vec<&str> = map
            .keys()
            .map(String::as_str)
            .filter(|key| !KEYS.contains(key))
            .collect();
        if !unknown.is_empty() {
            unknown.sort_unstable();
            return Err(format!("has unknown settings: {}", unknown.join(", ")));
        }
        let delimiter = choice(map.get("delimiter"), "delimiter", &DELIMITERS, ",")?;
        let header_rows = header_rows(map.get("header_rows"))?;
        let date_format = choice(
            map.get("date_format"),
            "date_format",
            &DATE_FORMATS,
            "YYYY-MM-DD",
        )?;
        let amount_sign = choice(
            map.get("amount_sign"),
            "amount_sign",
            &AMOUNT_SIGNS,
            "deposits_positive",
        )?;
        let decimal_separator = choice(
            map.get("decimal_separator"),
            "decimal_separator",
            &DECIMAL_SEPARATORS,
            ".",
        )?;
        let mut columns = Vec::with_capacity(COLUMNS.len());
        for key in COLUMNS {
            match column(map.get(key), header_rows) {
                Ok(None) if key == "date" || key == "description" => {
                    return Err(format!("{key} column is required"));
                }
                Ok(column) => columns.push(column),
                Err(()) if header_rows == 0 => {
                    return Err(format!(
                        "{key} must be a column position (1, 2, ...) because the file has no header row"
                    ));
                }
                Err(()) => return Err(format!("{key} must be a column header or position")),
            }
        }
        let [date, description, reference, amount, debit, credit, balance]: [Option<Column>; 7] =
            columns.try_into().expect("seven columns");
        match (&amount, &debit, &credit) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(
                    "names an amount column and debit or credit columns; use one or the other"
                        .into(),
                );
            }
            (Some(_), None, None) | (None, Some(_), Some(_)) => {}
            _ => return Err("needs an amount column, or both a debit and a credit column".into()),
        }
        Ok(Self {
            delimiter,
            header_rows,
            date,
            date_format,
            description,
            reference,
            amount,
            amount_sign,
            debit,
            credit,
            balance,
            decimal_separator,
        })
    }

    /// The mapping as the JSON object it is stored and audited as.
    pub(crate) fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("a mapping serializes")
    }

    /// A column's cell of a parsed row, by header text or 1-based position;
    /// an unmapped column, or a row too short to have it, is `None`.
    pub(crate) fn cell<'r>(
        row: &'r [String],
        headers: &[String],
        column: Option<&Column>,
    ) -> Option<&'r str> {
        match column? {
            Column::Position(position) => row.get(usize::try_from(*position).ok()? - 1),
            Column::Header(name) => headers
                .iter()
                .position(|header| header.trim() == name)
                .and_then(|index| row.get(index)),
        }
        .map(String::as_str)
    }

    /// The header names this mapping needs that `headers` does not have.
    pub(crate) fn missing_headers(&self, headers: &[String]) -> Vec<String> {
        let present: Vec<&str> = headers.iter().map(|header| header.trim()).collect();
        [
            &self.date,
            &self.description,
            &self.reference,
            &self.amount,
            &self.debit,
            &self.credit,
            &self.balance,
        ]
        .into_iter()
        .filter_map(|column| match column {
            Some(Column::Header(name)) if !present.contains(&name.as_str()) => Some(name.clone()),
            _ => None,
        })
        .collect()
    }
}

fn choice(
    value: Option<&Value>,
    key: &str,
    choices: &[&str],
    default: &str,
) -> Result<String, String> {
    match value {
        None | Some(Value::Null) => Ok(default.to_owned()),
        Some(Value::String(text)) if choices.contains(&text.as_str()) => Ok(text.clone()),
        _ => Err(format!(
            "{key} must be one of {}",
            choices
                .iter()
                .map(|choice| format!("{choice:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn header_rows(value: Option<&Value>) -> Result<u8, String> {
    match value {
        None | Some(Value::Null) => Ok(1),
        Some(value) => match value.as_i64() {
            Some(rows) if (0..=MAX_HEADER_ROWS).contains(&rows) => Ok(rows as u8),
            _ => Err(format!(
                "header_rows must be a whole number from 0 to {MAX_HEADER_ROWS}"
            )),
        },
    }
}

/// `Ok(None)` for an unmapped column, `Err` for a value that is neither a
/// header text (only when the file has a header row) nor a position.
fn column(value: Option<&Value>, header_rows: u8) -> Result<Option<Column>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::Number(number)) => match number.as_u64() {
            Some(position) if position > 0 => u32::try_from(position)
                .map(|position| Some(Column::Position(position)))
                .map_err(|_| ()),
            _ => Err(()),
        },
        Some(Value::String(text)) if header_rows > 0 => Ok(match text.trim() {
            "" => None,
            name => Some(Column::Header(name.to_owned())),
        }),
        _ => Err(()),
    }
}

/// Parses a date written in `format`; `None` for anything else, a real
/// calendar date in 1900 to 2200 only.
pub(crate) fn parse_date(text: &str, format: &str) -> Option<Date> {
    let parts: Vec<&str> = text
        .trim()
        .split(['-', '/', '.', ' '])
        .filter(|part| !part.is_empty())
        .collect();
    match (format, parts.as_slice()) {
        ("YYYY-MM-DD" | "YYYY/MM/DD", [y, m, d]) => date(y, m, d),
        ("DD/MM/YYYY" | "DD-MM-YYYY" | "DD.MM.YYYY", [d, m, y]) => date(y, m, d),
        ("MM/DD/YYYY", [m, d, y]) => date(y, m, d),
        ("YYYYMMDD", [whole]) if whole.len() == 8 && whole.is_ascii() => {
            date(&whole[..4], &whole[4..6], &whole[6..])
        }
        ("DD-MMM-YYYY", [d, month, y]) => month_name_date(y, month, d),
        ("DD-MMM-YY", [d, month, yy]) if yy.len() == 2 && yy.is_ascii() => {
            month_name_date(&format!("20{yy}"), month, d)
        }
        _ => None,
    }
}

fn month_name_date(y: &str, month: &str, d: &str) -> Option<Date> {
    let number = MONTHS
        .iter()
        .position(|name| *name == month.to_lowercase())?
        + 1;
    date(y, &number.to_string(), d)
}

fn date(y: &str, m: &str, d: &str) -> Option<Date> {
    let digits = |text: &str| -> Option<i64> {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (year, month, day) = (digits(y)?, digits(m)?, digits(d)?);
    if !(1900..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Date::new(year as i16, month as i8, day as i8).ok()
}

/// Parses an amount as banks write them: thousands separators, a leading or
/// trailing minus, parentheses, or a trailing `CR` (in) or `DR` (out) marker.
/// A missing or blank cell is `Ok(None)`.
pub(crate) fn parse_amount(text: Option<&str>, separator: &str) -> Result<Option<Decimal>, ()> {
    let Some(text) = text else { return Ok(None) };
    let trimmed = text.trim().to_uppercase();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let (negative, body) = sign_and_body(&trimmed);
    let digits: String = body
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\'')
        .collect();
    let digits = if separator == "," {
        digits.replace('.', "").replace(',', ".")
    } else {
        digits.replace(',', "")
    };
    let mut parts = digits.split('.');
    let shaped = |part: Option<&str>| {
        part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let whole = parts.next();
    let fraction = parts.next();
    if !shaped(whole) || (fraction.is_some() && !shaped(fraction)) || parts.next().is_some() {
        return Err(());
    }
    let amount = Decimal::from_str_exact(&digits).map_err(|_| ())?;
    Ok(Some(if negative { -amount } else { amount }))
}

fn sign_and_body(text: &str) -> (bool, &str) {
    if let Some(inner) = text
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        (true, inner)
    } else if let Some(rest) = text.strip_suffix("DR") {
        (true, rest.trim())
    } else if let Some(rest) = text.strip_suffix("CR") {
        (false, rest.trim())
    } else if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_suffix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest)
    } else {
        (false, text)
    }
}
