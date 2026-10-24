//! An imported bank statement as a record source documents support (Bilimbi
//! `Bilimbi.Finance.Reconciliation.StatementSource`): the bank's own file,
//! kept whole by `finance/attachments` as the statement is imported, so the
//! normalised lines can be checked against it for as long as the law asks.
//! It reads the statement on the connection it is handed, with the caller's
//! scope, so `finance.reconciliation.view` is also needed. A statement is
//! immutable from its import, so it is always final: its file is never
//! replaced or removed.
use crate::{contributions::VIEW, MODULE};
use bilibi_database::Error;
use bilibi_finance_attachments::{
    on_source_client, Source, SourceClient, SourceDefinition, SourceDocument, SourceFuture,
    SourceRefusal, SOURCES,
};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_tenancy::Scope;
use tokio_postgres::GenericClient;

/// The source id stored with each statement's attachment.
pub const STATEMENT_SOURCE: &str = "finance/reconciliation/statement";

struct StatementSource;

impl Source for StatementSource {
    fn label(&self) -> &'static str {
        "Bank statement"
    }
    fn view_capability(&self) -> &'static str {
        VIEW
    }
    fn document<'a>(
        &'a self,
        client: SourceClient<'a>,
        scope: &'a Scope,
        company_id: i64,
        document_id: i64,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            on_source_client!(client, |c| read(c, scope, company_id, document_id).await)
        })
    }
}

async fn read(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    statement_id: i64,
) -> Result<SourceDocument, SourceRefusal> {
    bilibi_company::authorize_target(client, scope, company_id, VIEW)
        .await
        .map_err(|error| match error {
            Error::Forbidden | Error::Unauthorized => SourceRefusal::Unauthorized,
            Error::NotFound => SourceRefusal::NotFound,
            other => SourceRefusal::Failed(other),
        })?;
    let row = client
        .query_opt(
            "SELECT statement_number,to_char(end_date,'YYYY-MM-DD') FROM finance_reconciliation_statements \
             WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
            &[&scope.tenant(), &company_id, &statement_id],
        )
        .await
        .map_err(Error::from)?
        .ok_or(SourceRefusal::NotFound)?;
    Ok(SourceDocument {
        label: format!("Statement {}", row.get::<_, i64>(0)),
        date: row.get::<_, &str>(1).parse().map_err(|_| Error::Internal)?,
        is_final: true,
    })
}

inventory::submit! { Contribution::new(SOURCES, STATEMENT_SOURCE,
Provenance { module: MODULE, source: file!(), line: line!() },
&SourceDefinition { source: &StatementSource }) }
