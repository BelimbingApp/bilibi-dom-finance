//! The file a source document is captured from, validated before any byte is
//! stored (Bilimbi `file_attributes/1`).
use crate::AttachmentError;

/// The largest file a page offers to upload, in bytes. Base Artifacts' own
/// `artifacts.max_bytes` setting is the limit that holds.
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// A file to keep: its name, its bytes and, optionally, its content type
/// (`application/octet-stream` when absent or blank).
#[derive(Clone)]
pub struct NewFile {
    pub file_name: String,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}
impl std::fmt::Debug for NewFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewFile")
            .field("file_name", &self.file_name)
            .field("bytes", &self.content.len())
            .field("content_type", &self.content_type)
            .finish()
    }
}

pub(crate) struct Valid {
    pub file_name: String,
    pub content_type: String,
}

pub(crate) fn validate(file: &NewFile) -> Result<Valid, AttachmentError> {
    let name = file.file_name.trim();
    if name.is_empty() || name.chars().count() > 255 {
        return Err(AttachmentError::InvalidFileName);
    }
    if file.content.is_empty() {
        return Err(AttachmentError::EmptyFile);
    }
    if file.content.len() > MAX_FILE_BYTES {
        return Err(AttachmentError::FileTooLarge);
    }
    let content_type = match file.content_type.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_CONTENT_TYPE,
        Some(other) => other,
    };
    if !content_type_shaped(content_type) {
        return Err(AttachmentError::InvalidContentType);
    }
    Ok(Valid {
        file_name: name.to_owned(),
        content_type: content_type.to_owned(),
    })
}

/// `type/subtype` of word characters, dots, plus signs and hyphens.
fn content_type_shaped(value: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
    };
    value.len() <= 255
        && value
            .split_once('/')
            .is_some_and(|(a, b)| word(a) && word(b))
}
