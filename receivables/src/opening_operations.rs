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
static OWNER: OnceLock<Arc<Receivables>> = OnceLock::new();
pub(super) fn bind(owner: Arc<Receivables>) -> bilibi_module_registry::Result<()> {
    OWNER
        .set(owner)
        .map_err(|_| RegistryError::Service("duplicate opening import binding".into()))
}
fn owner() -> Result<&'static Receivables> {
    OWNER
        .get()
        .map(Arc::as_ref)
        .ok_or_else(|| Error::Internal.into())
}
fn failure(e: ReceivableError) -> bilibi_agent_api::Error {
    crate::agent_operations::failure(e, "csv")
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
fn context_schema() -> Value {
    let mut value = target();
    let properties = value["properties"]
        .as_object_mut()
        .expect("schema properties");
    properties.insert("items_after".into(), json!({"type":"integer","minimum":0,"description":"Return imported items after this item id; pass the previous items_next."}));
    properties.insert("limit".into(), json!({"type":"integer","minimum":1,"maximum":500,"description":"Items per page, default 200."}));
    value
}
fn base_schema() -> Value {
    json!({"type":"object","required":["company_id","source_reference","currency_code","csv","series_id"],"properties":{"company_id":{"type":"integer","minimum":1},"source_reference":{"type":"string","maxLength":100},"currency_code":{"type":"string","maxLength":3},"csv":{"type":"string","maxLength":131072,"description":"UTF-8 CSV; obtain the exact required header from opening.context. Import the complete opening register, at most 500 rows."},"historical_control_account_codes":{"type":"array","maxItems":10000,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":50},"description":"Every historical receivables control account in the approved opening balance. The configured control is included automatically."},"override_reason":{"type":"string","maxLength":500},"series_id":{"type":"integer","minimum":1,"description":"Dedicated active invoice numbering series for imported opening items."}}})
}
fn schema() -> Value {
    let mut value = base_schema();
    let properties = value["properties"]
        .as_object_mut()
        .expect("schema properties");
    properties.insert(
        "part".into(),
        json!({"type":"integer","minimum":1,"maximum":200}),
    );
    properties.insert(
        "part_count".into(),
        json!({"type":"integer","minimum":1,"maximum":200}),
    );
    properties.insert("staged".into(),json!({"type":"boolean","description":"Reconcile retained source batches; pass an empty csv."}));
    properties.insert(
        "replaces_batch_id".into(),
        json!({"type":"integer","minimum":1}),
    );
    properties.insert("reason".into(), json!({"type":"string","maxLength":500}));
    value
}

fn stage_schema() -> Value {
    let mut value = schema();
    value["required"]
        .as_array_mut()
        .expect("schema required")
        .extend([json!("part"), json!("part_count")]);
    value
}
fn output() -> Value {
    json!({"type":"object"})
}
async fn fence(
    c: &dyn OperationClient,
    facts: &Value,
    consume: bool,
) -> bilibi_agent_api::Result<()> {
    let row=c.query_opt("SELECT encode(sha256(convert_to($1,'UTF8')),'hex'),COALESCE(NULLIF(current_setting('finance.receivables_reviews',true),''),'[]')::jsonb",&[&facts.to_string()]).await?.ok_or(Error::Internal)?;
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
        "SELECT set_config('finance.receivables_reviews',$1,true)",
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
                .opening_context(c, s, company(input).map_err(failure)?, input)
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
            let resolved = owner()
                .map_err(failure)?
                .resolve_opening(c, s, company(input).map_err(failure)?, input)
                .await
                .map_err(failure)?;
            let prepared = owner()
                .map_err(failure)?
                .prepare_opening(c, s, company(input).map_err(failure)?, &resolved)
                .await
                .map_err(failure)?;
            fence(c, &json!({"input":input,"review":prepared.review}), false).await?;
            Ok(json!({"changes":prepared.review}).into())
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
            let resolved = owner
                .resolve_opening(tx.sql(), s, company, input)
                .await
                .map_err(failure)?;
            let prepared = owner
                .prepare_opening(tx.sql(), s, company, &resolved)
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
            Ok(json!({"record":record,"path":"/finance/receivables/opening"}).into())
        })
    }
}
struct Stage;
impl WriteHandler for Stage {
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
                .prepare_opening_stage(c, s, company(input).map_err(failure)?, input)
                .await
                .map_err(failure)?;
            fence(c, &json!({"input":input,"review":prepared}), false).await?;
            Ok(json!({"changes":prepared}).into())
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
                .prepare_opening_stage(tx.sql(), s, company, input)
                .await
                .map_err(failure)?;
            fence(tx.sql(), &json!({"input":input,"review":prepared}), true).await?;
            let record = owner
                .stage_opening_on(tx, s, company, input)
                .await
                .map_err(failure)?;
            Ok(json!({"record":record,"path":"/finance/receivables/opening"}).into())
        })
    }
}
inventory::submit! {Contribution::new(DEFINITIONS,"finance/receivables.opening",Provenance{module:MODULE,source:file!(),line:line!()},&Definitions{operations:&[
 Operation{key:"finance.receivables.opening.context",title:"Read opening import context",summary:"Read the approved conversion entry, source balances, CSV header and retained import batch for the current company.",keywords:&["conversion"],capability:VIEW,input:context_schema,output,handler:Handler::Read(&Context),channels:Channels::Every,step_up:&[]},
 Operation{key:"finance.receivables.opening.stage",title:"Stage opening source batch",summary:"Retain one immutable source batch for a resumable register. Identical retries are safe. Reconcile only after all batches are committed, with approval independent of every staging preparer.",keywords:&["conversion"],capability:ISSUE,input:stage_schema,output,handler:Handler::SafeWrite(&Stage),channels:Channels::Every,step_up:&[]},
 Operation{key:"finance.receivables.opening.import",title:"Opening invoices",summary:"Preview a complete opening register and exact reconciliation, then approve once. Retain source evidence without a new acquisition, supply or tax posting. Identical retries return the retained batch. Requires approval independent of the opening source preparer.",keywords:&["conversion"],capability:ISSUE,input:schema,output,handler:Handler::SafeWrite(&Import),channels:Channels::Every,step_up:&[]}
],guides:&[bilibi_agent_api::Guide {key:"finance.receivables.opening",title:"Opening invoices",summary:"Convert source registers without duplicating opening postings.",keywords:&["conversion"],capability:VIEW,body:include_str!("../opening-guide.md")}]})}
