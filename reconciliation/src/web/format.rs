//! Bilimbi `Finance.Reconciliation.Web.Format`: refusal copy and amounts.
use crate::ReconciliationError as E;
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::Money;
use rust_decimal::Decimal;

pub(super) fn told<T>(result: crate::Result<T>) -> Result<std::result::Result<T, String>> {
    match result {
        Ok(value) => Ok(Ok(value)),
        Err(E::Failed(Error::Forbidden | Error::Unauthorized)) => Ok(Err(super::DENIED.into())),
        Err(E::Failed(error)) => Err(error),
        Err(error) => Ok(Err(refusal(&error))),
    }
}
pub(super) fn refusal(error: &E) -> String {
    match error {
        E::Unauthorized => super::DENIED.into(),
        E::CompanyArchived => "The company is archived; nothing can be changed.".into(),
        E::CompanyNotFound => "The company could not be found.".into(),
        E::BookNotOpen => "The company's ledger book is not open yet.".into(),
        E::BankAccountNotFound => "The bank account no longer exists.".into(),
        E::BankAccountInactive => "The bank account is inactive.".into(),
        E::StatementNotFound => "The statement no longer exists.".into(),
        E::InvalidFileName => "The file needs a name.".into(),
        E::NoLines => "The file holds no statement lines.".into(),
        E::FileTooLarge => "The file is larger than the import allows.".into(),
        E::NotUtf8 => "The file is not UTF-8 text.".into(),
        E::InvalidOption => "The request was malformed.".into(),
        E::Invalid(errors) => format!("Refused: {}.", errors.iter().map(|e|format!("{} {}", e.field.replace('_', " "), e.message)).collect::<Vec<_>>().join("; ")),
        E::DuplicateFile { statement_number } => format!("This file was already imported as statement {statement_number}."),
        E::DuplicateLines(lines) => {
            let shown = lines.iter().take(5).map(|l|format!("row {} (statement {}, line {})", l.row, l.statement_number, l.statement_line_number)).collect::<Vec<_>>().join(", ");
            let more = if lines.len() > 5 { format!(" and {} more", lines.len()-5) } else { String::new() };
            format!("Nothing was imported: {} lines are already on file: {shown}{more}.", lines.len())
        }
        E::InvalidLines(lines) => {
            let shown = lines.iter().take(5).map(|l|format!("row {}: {}", l.row, l.message)).collect::<Vec<_>>().join("; ");
            let more = if lines.len() > 5 {format!("; and {} more", lines.len()-5)} else {String::new()};
            format!("Nothing was imported: {shown}{more}.")
        }
        E::MissingHeaders(headers) => format!("The file has no column named {}; check the bank account's import mapping.", headers.iter().map(|h|format!("“{h}”")).collect::<Vec<_>>().join(", ")),
        E::TooManyLines(max) => format!("The file holds more than {max} lines; split it."),
        E::RunningBalanceMismatch(row) => format!("Row {row}'s running balance does not follow from the amounts; check the amount sign or the balance column."),
        E::UnterminatedQuote(row) => format!("Row {row} opens a quote it never closes."),
        E::StrayQuote(row) => format!("Row {row} has a quote in the middle of a value."),
        E::Attachment(bilibi_finance_attachments::AttachmentError::RetentionNotConfigured) => "Source documents cannot be kept yet; an operator must set up the file storage.".into(),
        other => other.to_string(),
    }
}
pub(super) fn money(value: Option<Money>, units: u8) -> String {
    value.map_or_else(|| "—".into(), |value| amount(value.amount(), units))
}
/// A signed amount with thousands separators, rounded to the book's minor
/// units (Bilimbi `Format.money`).
pub(super) fn amount(value: Decimal, units: u8) -> String {
    let mut value = value.round_dp(u32::from(units));
    // Money is normalized; Bilimbi shows every minor unit (12.50).
    value.rescale(u32::from(units));
    let text = value.abs().to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let grouped = whole
        .chars()
        .enumerate()
        .fold(String::new(), |mut out, (i, c)| {
            if i > 0 && (whole.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
            out
        });
    format!(
        "{}{}{}{}",
        if value.is_sign_negative() && !value.is_zero() {
            "-"
        } else {
            ""
        },
        grouped,
        if fraction.is_empty() { "" } else { "." },
        fraction
    )
}
