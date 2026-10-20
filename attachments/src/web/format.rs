//! How the attachments pages word refusals, sizes and hashes (Bilimbi
//! `Finance.Attachments.Web.Format`).
use crate::AttachmentError;
use bilibi_database::Error;

/// The sentence a refusal is told in, or the failure to propagate: a
/// database or capture failure is never worded to a person.
pub(crate) fn refusal(error: AttachmentError) -> Result<String, Error> {
    use AttachmentError as E;
    Ok(match error {
        E::Unauthorized => "You are not allowed to do that.".into(),
        E::CompanyNotFound => "This company cannot be read with your current access.".into(),
        E::CompanyArchived => "This company is archived; nothing more is recorded.".into(),
        E::NotFound => "There is no such record for this company.".into(),
        E::AttachmentNotFound => "There is no such attachment for this company.".into(),
        E::AttachmentNotStanding => "That attachment was already replaced or removed.".into(),
        E::DocumentFinal => "The record is final; its attachments are kept as they are.".into(),
        E::InvalidFileName => "The file needs a name of at most 255 characters.".into(),
        E::EmptyFile => "The file is empty.".into(),
        E::FileTooLarge => "The file is larger than this page accepts.".into(),
        E::InvalidContentType => "The file's type could not be read.".into(),
        E::InvalidSize => "The file is larger than Base Artifacts accepts.".into(),
        E::ReasonRequired => "Give a reason.".into(),
        E::RetentionNotLengthened => "Retention is only ever lengthened.".into(),
        E::RetentionNotExtended => {
            "The new retain-until must be later than the current one.".into()
        }
        E::SourceUnavailable => "The module that owns this record is not mounted.".into(),
        E::InvalidSource => "That kind of record cannot take attachments.".into(),
        refused @ (E::RetentionNotConfigured | E::StorageFolderMissing(_)) => refused.to_string(),
        E::Invalid(errors) => errors
            .iter()
            .map(|e| format!("{} {}", e.field.replace('_', " "), e.message))
            .collect::<Vec<_>>()
            .join("; "),
        E::Failed(error) => return Err(error),
        // Bilimbi `"The action was refused: #{inspect(other)}."`: the atom.
        E::CaptureMismatch => "The action was refused: :capture_mismatch.".into(),
        E::DocumentNotFinal => "The action was refused: :document_not_final.".into(),
        E::RetentionOutOfRange => "The action was refused: :invalid_retention.".into(),
        E::AlreadyOnHold => "The action was refused: :already_on_hold.".into(),
        E::NotOnHold => "The action was refused: :not_on_hold.".into(),
        E::InvalidOption => "The action was refused: :invalid_option.".into(),
        E::Storage(code) => format!("The action was refused: :{code}."),
        other => format!("The action was refused: {other}."),
    })
}

/// A byte count a person reads: `12.3 KB`, `4.0 MB`.
pub(crate) fn bytes(size: i64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

/// The first and last eight characters of a SHA-256, for a table cell; the
/// title holds it whole.
pub(crate) fn short_hash(hash: &str) -> String {
    if hash.len() == 64 && hash.is_ascii() {
        format!("{}…{}", &hash[..8], &hash[56..])
    } else {
        hash.to_owned()
    }
}

/// The date of a retain-until, which is the end of that day in UTC.
pub(crate) fn retain_until(until: jiff::Timestamp) -> String {
    until.to_zoned(jiff::tz::TimeZone::UTC).date().to_string()
}
