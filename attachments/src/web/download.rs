//! `/finance/attachments/{id}/download` (Bilimbi
//! `Web.DownloadController`). Every request reads through the facade, which
//! re-asks every view capability, re-reads the record the file supports and
//! reads the bytes through Base Artifacts; the delivery headers (attachment,
//! no sniffing, no caching, sandboxed) come from `FileBody::headers`.
use crate::{AttachmentError, Attachments};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use std::sync::Arc;

pub(crate) struct Downloads {
    pub attachments: Arc<Attachments>,
}

/// The session's current company (Bilibi ADR 09); it grants nothing.
pub(crate) fn company_of(scope: &Scope) -> Option<i64> {
    scope.current_company().map(|id| id.get())
}

impl Download<Scope, Error> for Downloads {
    fn serve<'a>(&'a self, request: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            if request.operation != "download" {
                return Err(Error::NotFound);
            }
            let id: i64 = request
                .path
                .get("id")
                .and_then(|id| id.parse().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let company_id = company_of(request.scope).ok_or(Error::NotFound)?;
            match self
                .attachments
                .read_attachment(request.scope, company_id, id)
                .await
            {
                Ok(file) => Ok(FileBody::attachment(
                    file.bytes,
                    &file.content_type,
                    &file.file_name,
                )),
                Err(AttachmentError::Unauthorized) => Err(Error::Forbidden),
                Err(AttachmentError::Failed(error)) => Err(error),
                Err(_) => Err(Error::NotFound),
            }
        })
    }
}
