//! Bilimbi Finance.Sst.Web.ExportController: a return as a CSV download, or
//! as a printable page shown in the tab. Every request reads the return
//! through the facade, which re-authorizes; the delivery headers (no caching,
//! no sniffing, no scripts) come from `FileBody::headers`.
use super::company;
use crate::{ExportFormat, Sst, SstError};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use std::sync::Arc;

pub(crate) struct Downloads {
    pub sst: Arc<Sst>,
}

impl Download<Scope, Error> for Downloads {
    fn serve<'a>(&'a self, request: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            let format = match request.operation {
                "csv" => ExportFormat::Csv,
                "print" => ExportFormat::Html,
                _ => return Err(Error::NotFound),
            };
            let id = bilibi_ui::params::positive_id(request.path.get("id").map(String::as_str))
                .ok_or(Error::NotFound)?;
            let company_id = company(request.scope).ok_or(Error::Forbidden)?;
            match self
                .sst
                .export_return(request.scope, company_id, id, format)
                .await
            {
                Ok(export) => {
                    let bytes = export.content.into_bytes();
                    Ok(match format {
                        ExportFormat::Csv => {
                            FileBody::attachment(bytes, export.content_type, &export.filename)
                        }
                        ExportFormat::Html => {
                            FileBody::inline(bytes, export.content_type, &export.filename)
                        }
                    })
                }
                Err(SstError::Unauthorized) => Err(Error::Forbidden),
                Err(SstError::Failed(error)) => Err(error),
                Err(_) => Err(Error::NotFound),
            }
        })
    }
}
