//! Bilimbi.Finance.Payables.Attachments.Bill: read on the caller's connection.
use crate::*;
use bilibi_finance_attachments::{
    on_source_client, Source, SourceClient, SourceDefinition, SourceDocument, SourceFuture,
    SourceRefusal, SOURCES,
};
use bilibi_module_registry::{Contribution, Provenance};
struct Documents;
impl Source for Documents {
    fn label(&self) -> &'static str {
        "Payables document"
    }
    fn view_capability(&self) -> &'static str {
        VIEW
    }
    fn document<'a>(
        &'a self,
        c: SourceClient<'a>,
        s: &'a Scope,
        company: i64,
        id: i64,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            on_source_client!(c, |sql| {
                authorize(sql, s, company, VIEW)
                    .await
                    .map_err(|e| match e {
                        PayablesError::Unauthorized => SourceRefusal::Unauthorized,
                        PayablesError::CompanyNotFound => SourceRefusal::NotFound,
                        PayablesError::Failed(e) => SourceRefusal::Failed(e),
                        _ => SourceRefusal::Failed(Error::Invalid),
                    })?;
                let d = store::document(sql, s.tenant(), company, id)
                    .await?
                    .ok_or(SourceRefusal::NotFound)?;
                Ok(SourceDocument {
                    label: d.label(),
                    date: d.document_date,
                    is_final: d.status != "draft",
                })
            })
        })
    }
}
inventory::submit! {Contribution::new(SOURCES,DOCUMENT_SOURCE,Provenance {module:MODULE,source:file!(),line:line!()},&SourceDefinition {source:&Documents})}
