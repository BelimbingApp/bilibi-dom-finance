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

// Bilimbi.Finance.Payables.Attachments.Payment: posted payments are final records.
struct Payments;
impl Source for Payments {
    fn label(&self) -> &'static str { "Payables payment" }
    fn view_capability(&self) -> &'static str { VIEW }
    fn document<'a>(&'a self,c:SourceClient<'a>,s:&'a Scope,company:i64,id:i64)->SourceFuture<'a> {
        Box::pin(async move {
            on_source_client!(c, |sql| {
                authorize(sql,s,company,VIEW).await.map_err(|e| match e {
                    PayablesError::Unauthorized=>SourceRefusal::Unauthorized,
                    PayablesError::CompanyNotFound=>SourceRefusal::NotFound,
                    PayablesError::Failed(e)=>SourceRefusal::Failed(e),
                    _=>SourceRefusal::Failed(Error::Invalid),
                })?;
                let r=sql.query_opt("SELECT payment_number,payment_date::text FROM finance_payables_payments WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&s.tenant(),&company,&id]).await?.ok_or(SourceRefusal::NotFound)?;
                Ok(SourceDocument {label:format!("Payment {}",r.get::<_,i64>(0)),date:r.get::<_,String>(1).parse().map_err(|_|SourceRefusal::Failed(Error::Invalid))?,is_final:true})
            })
        })
    }
}
inventory::submit! {Contribution::new(SOURCES,PAYMENT_SOURCE,Provenance {module:MODULE,source:file!(),line:line!()},&SourceDefinition {source:&Payments})}
