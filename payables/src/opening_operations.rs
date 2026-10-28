//! Conversion contracts share the owner path with the UI. The transaction-local
//! review fence prevents earlier plan steps from changing reviewed opening facts.
use crate::*;
use bilibi_agent_api::{
    Channels, Definitions, Handler, Operation, OperationClient, OperationFuture, ReadHandler,
    WriteHandler, DEFINITIONS,
};
use bilibi_module_registry::{Contribution, Provenance, RegistryError};
use serde_json::{json, Value};
use std::sync::OnceLock;
static OWNER: OnceLock<Arc<Payables>> = OnceLock::new();
pub(super) fn bind(owner: Arc<Payables>) -> bilibi_module_registry::Result<()> {
    OWNER
        .set(owner)
        .map_err(|_| RegistryError::Service("duplicate opening import binding".into()))
}
fn owner() -> Result<&'static Payables> {
    OWNER
        .get()
        .map(Arc::as_ref)
        .ok_or_else(|| Error::Internal.into())
}
fn failure(e: PayablesError) -> bilibi_agent_api::Error {
    crate::agent_operations::error(e, "csv")
}
fn company(input: &Value) -> Result<i64> {
    input["company_id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("company_id", "must identify the current company"))
}
fn target() -> Value {
    json!({"type":"object","required":["company_id"],"properties":{"company_id":{"type":"integer","minimum":1}}})
}
fn schema() -> Value {
    json!({"type":"object","required":["company_id","source_reference","currency_code","csv"],"properties":{"company_id":{"type":"integer","minimum":1},"source_reference":{"type":"string","maxLength":100},"currency_code":{"type":"string","maxLength":3},"csv":{"type":"string","maxLength":131072,"description":"UTF-8 CSV; obtain the exact required header from opening.context. Import the complete opening register, at most 500 rows."},"override_reason":{"type":"string","maxLength":500}}})
}
fn output() -> Value {
    json!({"type":"object"})
}
async fn fence(
    c: &dyn OperationClient,
    facts: &Value,
    consume: bool,
) -> bilibi_agent_api::Result<()> {
    let row=c.query_opt("SELECT encode(sha256(convert_to($1,'UTF8')),'hex'),COALESCE(NULLIF(current_setting('finance.payables_reviews',true),''),'[]')::jsonb",&[&facts.to_string()]).await?.ok_or(Error::Internal)?;
    let digest: String = row.get(0);
    let mut pending: Vec<String> =
        serde_json::from_value(row.get(1)).map_err(|_| Error::Internal)?;
    if consume {
        if pending.first() != Some(&digest) {
            return Err(bilibi_agent_api::Error::StalePlan);
        }
        pending.remove(0);
    } else {
        pending.push(digest);
    }
    c.query_opt(
        "SELECT set_config('finance.payables_reviews',$1,true)",
        &[&json!(pending).to_string()],
    )
    .await?;
    Ok(())
}
struct Context;
impl ReadHandler for Context {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn call<'a>(
        &'a self,
        c: &'a dyn OperationClient,
        s: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            Ok(owner()
                .map_err(failure)?
                .opening_context(c, s, company(input).map_err(failure)?)
                .await
                .map_err(failure)?
                .into())
        })
    }
}
struct Import;
impl WriteHandler for Import {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        c: &'a dyn OperationClient,
        s: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let prepared = owner()
                .map_err(failure)?
                .prepare_opening(c, s, company(input).map_err(failure)?, input)
                .await
                .map_err(failure)?;
            fence(c, &json!({"input":input,"review":prepared.review}), false).await?;
            Ok(prepared.review.into())
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        s: &'a Scope,
        input: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let owner = owner().map_err(failure)?;
            let company = company(input).map_err(failure)?;
            let prepared = owner
                .prepare_opening(tx.sql(), s, company, input)
                .await
                .map_err(failure)?;
            fence(
                tx.sql(),
                &json!({"input":input,"review":prepared.review}),
                true,
            )
            .await?;
            let record = owner
                .import_opening_on(tx, s, company, input)
                .await
                .map_err(failure)?;
            Ok(json!({"record":record,"path":"/finance/payables/opening"}).into())
        })
    }
}
inventory::submit! {Contribution::new(DEFINITIONS,"finance/payables.opening",Provenance{module:MODULE,source:file!(),line:line!()},&Definitions{operations:&[
 Operation{key:"finance.payables.opening.context",title:"Read opening import context",summary:"Read the approved conversion entry, source balances, CSV header and retained import batch for the current company.",keywords:&["conversion"],capability:VIEW,input:target,output,handler:Handler::Read(&Context),channels:Channels::Every,step_up:&[]},
 Operation{key:"finance.payables.opening.import",title:"Opening bills",summary:"Preview a complete opening register and exact reconciliation, then approve once. Retain source evidence without a new acquisition, supply or tax posting. Identical retries return the retained batch. Requires approval independent of the opening source preparer.",keywords:&["conversion"],capability:POST_DOCUMENTS,input:schema,output,handler:Handler::SafeWrite(&Import),channels:Channels::Every,step_up:&[]}
],guides:&[bilibi_agent_api::Guide {key:"finance.payables.opening",title:"Opening bills",summary:"Convert source registers without duplicating opening postings.",keywords:&["conversion"],capability:VIEW,body:include_str!("../opening-guide.md")}]})}
