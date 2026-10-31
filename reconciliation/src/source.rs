//! Reconciliation records whose source documents are kept by
//! `finance/attachments`. They read on the connection they are handed, with
//! the caller's scope, so `finance.reconciliation.view` is also needed. Both
//! statements and opening bridges are immutable, so they are always final.
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
/// The source id stored with each opening bridge's attachments.
pub const OPENING_BRIDGE_SOURCE: &str = "finance/reconciliation/opening_bridge";

struct StatementSource;

impl Source for StatementSource {
    fn label(&self) -> &'static str {
        "Bank statement"
    }
    fn view_capabilities(&self) -> &'static [&'static str] {
        &[VIEW]
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
    bilibi_company::authorize_work_target(client, scope, company_id, VIEW)
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

struct OpeningBridgeSource;

impl Source for OpeningBridgeSource {
    fn label(&self) -> &'static str {
        "Opening bank bridge"
    }
    fn view_capabilities(&self) -> &'static [&'static str] {
        &[VIEW]
    }
    fn document<'a>(
        &'a self,
        client: SourceClient<'a>,
        scope: &'a Scope,
        company_id: i64,
        document_id: i64,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            on_source_client!(client, |c| read_opening_bridge(
                c,
                scope,
                company_id,
                document_id
            )
            .await)
        })
    }
}

async fn read_opening_bridge(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company_id: i64,
    bridge_id: i64,
) -> Result<SourceDocument, SourceRefusal> {
    bilibi_company::authorize_work_target(client, scope, company_id, VIEW)
        .await
        .map_err(|error| match error {
            Error::Forbidden | Error::Unauthorized => SourceRefusal::Unauthorized,
            Error::NotFound => SourceRefusal::NotFound,
            other => SourceRefusal::Failed(other),
        })?;
    let row = client
        .query_opt(
            "SELECT source_reference,to_char(conversion_date,'YYYY-MM-DD') \
             FROM finance_reconciliation_opening_bridges \
             WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
            &[&scope.tenant(), &company_id, &bridge_id],
        )
        .await
        .map_err(Error::from)?
        .ok_or(SourceRefusal::NotFound)?;
    Ok(SourceDocument {
        label: format!("Opening bridge {}", row.get::<_, String>(0)),
        date: row.get::<_, &str>(1).parse().map_err(|_| Error::Internal)?,
        is_final: true,
    })
}

inventory::submit! { Contribution::new(SOURCES, OPENING_BRIDGE_SOURCE,
Provenance { module: MODULE, source: file!(), line: line!() },
&SourceDefinition { source: &OpeningBridgeSource }) }
