//! Private owned SQL and frozen evidence fields, mirroring FixedAssets.Evidence.
use crate::*;
use bilibi_database::{Change, Result as DbResult};
use bilibi_finance_ledger::MinorUnits;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) enum Table {
    OpeningStage,
    OpeningDecision,
    OpeningBatch,
    OpeningItem,
    Category,
    Revision,
    Asset,
    Run,
    Item,
    Exception,
    Event,
}
impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::OpeningStage => "finance_fixed_assets_opening_stages",
            Self::OpeningDecision => "finance_fixed_assets_opening_decisions",
            Self::OpeningBatch => "finance_fixed_assets_opening_batches",
            Self::OpeningItem => "finance_fixed_assets_opening_items",
            Self::Category => "finance_fixed_assets_categories",
            Self::Revision => "finance_fixed_assets_category_revisions",
            Self::Asset => "finance_fixed_assets_assets",
            Self::Run => "finance_fixed_assets_runs",
            Self::Item => "finance_fixed_assets_run_items",
            Self::Exception => "finance_fixed_assets_run_exceptions",
            Self::Event => "finance_fixed_assets_events",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::OpeningStage => "finance.fixed-assets.opening_stage",
            Self::OpeningDecision => "finance.fixed-assets.opening_decision",
            Self::OpeningBatch => "finance.fixed-assets.opening_batch",
            Self::OpeningItem => "finance.fixed-assets.opening_item",
            Self::Category => "finance.fixed-assets.category",
            Self::Revision => "finance.fixed-assets.category_revision",
            Self::Asset => "finance.fixed-assets.asset",
            Self::Run => "finance.fixed-assets.run",
            Self::Item => "finance.fixed-assets.run_item",
            Self::Exception => "finance.fixed-assets.run_exception",
            Self::Event => "finance.fixed-assets.event",
        }
    }
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::OpeningStage => &[
                "id",
                "tenant_id",
                "company_id",
                "source_reference",
                "part",
                "payload",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::OpeningDecision => &[
                "id",
                "tenant_id",
                "company_id",
                "batch_id",
                "replaces_batch_id",
                "reason",
                "effective_date",
                "review",
                "approval",
                "state",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::OpeningBatch => &[
                "id",
                "tenant_id",
                "company_id",
                "source_reference",
                "source_csv",
                "currency_code",
                "conversion_date",
                "entry_id",
                "entry_number",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::OpeningItem => &[
                "id",
                "tenant_id",
                "company_id",
                "batch_id",
                "source_id",
                "asset_id",
                "source_facts",
            ],
            Self::Category => &[
                "id",
                "tenant_id",
                "company_id",
                "name",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Revision => &[
                "id",
                "tenant_id",
                "company_id",
                "category_id",
                "cost_account_id",
                "accumulated_account_id",
                "expense_account_id",
                "gain_account_id",
                "loss_account_id",
                "method",
                "life_months",
                "annual_rate",
                "reason",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Run => &[
                "id",
                "tenant_id",
                "company_id",
                "period",
                "entry_id",
                "entry_number",
                "total",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Item => &[
                "id",
                "tenant_id",
                "company_id",
                "run_id",
                "asset_id",
                "amount",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Exception => &[
                "id",
                "tenant_id",
                "company_id",
                "run_id",
                "asset_id",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Event => &[
                "id",
                "tenant_id",
                "company_id",
                "event_number",
                "asset_id",
                "run_id",
                "action",
                "effective_date",
                "entry_id",
                "entry_number",
                "reverses_event_id",
                "proceeds",
                "accumulated",
                "reason",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
            Self::Asset => &[
                "id",
                "tenant_id",
                "company_id",
                "asset_number",
                "category_revision_id",
                "description",
                "acquisition_date",
                "in_service_date",
                "cost",
                "residual_value",
                "method",
                "life_months",
                "annual_rate",
                "entry_id",
                "entry_number",
                "entry_line_number",
                "bill_id",
                "bill_line_number",
                "recorded_by_user_id",
                "recorded_by_impersonator_id",
                "recorded_at",
            ],
        }
    }
}
pub(crate) fn identity(s: &Scope, company: i64) -> Value {
    json!({"tenant_id":s.tenant(),"company_id":company,"recorded_by_user_id":s.user(),"recorded_by_impersonator_id":s.impersonator().map(|p|p.user())})
}
pub(crate) async fn insert(
    c: &(impl GenericClient + Sync),
    t: Table,
    mut value: Value,
) -> DbResult<(Value, Change)> {
    if value["id"].is_null() {
        value["id"] = reserve(c, t).await?.into();
    }
    let name = t.name();
    let r = c.query_one(&format!("INSERT INTO {name} SELECT (jsonb_populate_record(NULL::{name}, $1::jsonb)).* RETURNING to_jsonb({name})"), &[&value]).await?;
    let value: Value = r.get(0);
    Ok((
        value.clone(),
        Change {
            kind: t.kind().into(),
            id: int(&value, "id")?,
            old: Value::Null,
            new: value,
        },
    ))
}
pub(crate) async fn reserve(c: &(impl GenericClient + Sync), t: Table) -> DbResult<i64> {
    Ok(c.query_one(
        "SELECT nextval(pg_get_serial_sequence($1,'id'))",
        &[&t.name()],
    )
    .await?
    .get(0))
}
pub(crate) async fn all(
    c: &dyn OperationClient,
    t: Table,
    tenant: i64,
    company: i64,
) -> DbResult<Vec<Value>> {
    Ok(c.query(
        &format!(
            "SELECT to_jsonb(r) FROM {} r WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",
            t.name()
        ),
        &[&tenant, &company],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect())
}
pub(crate) async fn raw(
    c: &dyn OperationClient,
    t: Table,
    tenant: i64,
    company: i64,
    id: i64,
) -> DbResult<Option<Value>> {
    Ok(c.query_opt(
        &format!(
            "SELECT to_jsonb(r) FROM {} r WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
            t.name()
        ),
        &[&tenant, &company, &id],
    )
    .await?
    .map(|r| r.get(0)))
}
pub(crate) fn int(v: &Value, key: &str) -> DbResult<i64> {
    v[key].as_i64().ok_or(Error::Invalid)
}
pub(crate) fn string(v: &Value, key: &str) -> DbResult<String> {
    v[key].as_str().map(str::to_owned).ok_or(Error::Invalid)
}
pub(crate) fn decimal(v: &Value, key: &str) -> DbResult<Decimal> {
    let text = v[key]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v[key].to_string());
    Decimal::from_str_exact(&text).map_err(|_| Error::Invalid)
}
fn assumptions(v: &Value) -> DbResult<Assumptions> {
    Ok(Assumptions {
        method: match v["method"].as_str() {
            Some("straight_line") => Method::StraightLine,
            Some("reducing_balance") => Method::ReducingBalance,
            _ => return Err(Error::Invalid),
        },
        life_months: i32::try_from(int(v, "life_months")?).map_err(|_| Error::Invalid)?,
        annual_rate: if v["annual_rate"].is_null() {
            None
        } else {
            Some(decimal(v, "annual_rate")?)
        },
    })
}
pub(crate) fn asset(v: &Value, units: MinorUnits) -> DbResult<Asset> {
    let money = |k| Money::new(decimal(v, k)?, units).map_err(|_| Error::Invalid);
    Ok(Asset {
        id: int(v, "id")?,
        asset_number: int(v, "asset_number")?,
        category_revision_id: int(v, "category_revision_id")?,
        description: string(v, "description")?,
        acquisition_date: string(v, "acquisition_date")?
            .parse()
            .map_err(|_| Error::Invalid)?,
        in_service_date: string(v, "in_service_date")?
            .parse()
            .map_err(|_| Error::Invalid)?,
        cost: money("cost")?,
        residual_value: money("residual_value")?,
        assumptions: assumptions(v)?,
        entry_id: int(v, "entry_id")?,
        entry_number: int(v, "entry_number")?,
        entry_line_number: v["entry_line_number"]
            .as_i64()
            .map(i32::try_from)
            .transpose()
            .map_err(|_| Error::Invalid)?,
        opening_date: v["opening_date"]
            .as_str()
            .map(str::parse)
            .transpose()
            .map_err(|_| Error::Invalid)?,
        opening_accumulated: if v["opening_accumulated"].is_null() {
            Money::ZERO
        } else {
            money("opening_accumulated")?
        },
        bill_id: v["bill_id"].as_i64(),
        bill_line_number: v["bill_line_number"]
            .as_i64()
            .map(i32::try_from)
            .transpose()
            .map_err(|_| Error::Invalid)?,
        recorded_by_user_id: int(v, "recorded_by_user_id")?,
        recorded_by_impersonator_id: v["recorded_by_impersonator_id"].as_i64(),
        recorded_at: string(v, "recorded_at")?,
    })
}
pub(crate) async fn revision(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<CategoryRevision> {
    let v = raw(c, Table::Revision, tenant, company, id)
        .await?
        .ok_or(FixedAssetsError::NotFound)?;
    let category_id = int(&v, "category_id")?;
    let category = raw(c, Table::Category, tenant, company, category_id)
        .await?
        .ok_or(FixedAssetsError::NotFound)?;
    Ok(CategoryRevision {
        id,
        category_id,
        name: string(&category, "name")?,
        defaults: RevisionInput {
            cost_account_id: int(&v, "cost_account_id")?,
            accumulated_account_id: int(&v, "accumulated_account_id")?,
            expense_account_id: int(&v, "expense_account_id")?,
            gain_account_id: int(&v, "gain_account_id")?,
            loss_account_id: int(&v, "loss_account_id")?,
            assumptions: assumptions(&v)?,
            reason: string(&v, "reason")?,
        },
        recorded_by_user_id: int(&v, "recorded_by_user_id")?,
        recorded_by_impersonator_id: v["recorded_by_impersonator_id"].as_i64(),
        recorded_at: string(&v, "recorded_at")?,
    })
}
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}
pub(crate) fn datasets() -> Vec<&'static str> {
    vec![
        "opening_stage",
        "opening_decision",
        "opening_batch",
        "opening_item",
        "asset",
        "category",
        "category_revision",
        "event",
        "run",
        "run_exception",
        "run_item",
    ]
}
pub(crate) async fn page(
    c: &(impl GenericClient + Sync),
    tenant: i64,
    company: i64,
    dataset: &str,
    limit: Option<i64>,
    after: Option<i64>,
) -> Result<EvidencePage> {
    let t = match dataset {
        "opening_stage" => Table::OpeningStage,
        "opening_decision" => Table::OpeningDecision,
        "opening_batch" => Table::OpeningBatch,
        "opening_item" => Table::OpeningItem,
        "asset" => Table::Asset,
        "category" => Table::Category,
        "category_revision" => Table::Revision,
        "event" => Table::Event,
        "run" => Table::Run,
        "run_exception" => Table::Exception,
        "run_item" => Table::Item,
        _ => return Err(FixedAssetsError::InvalidOption),
    };
    let limit = limit.unwrap_or(500);
    let after = after.unwrap_or(0);
    if !(1..=500).contains(&limit) || after < 0 {
        return Err(FixedAssetsError::InvalidOption);
    }
    let columns = t
        .fields()
        .iter()
        .map(|f| {
            if *f == "recorded_at" {
                format!("to_char(r.{f},'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS {f}")
            } else if f.ends_with("_date") {
                format!("to_char(r.{f},'YYYY-MM-DD') AS {f}")
            } else if matches!(
                *f,
                "cost"
                    | "residual_value"
                    | "annual_rate"
                    | "total"
                    | "amount"
                    | "proceeds"
                    | "accumulated"
            ) {
                format!("r.{f}::text AS {f}")
            } else {
                format!("r.{f}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let rows=c.query(&format!("SELECT to_jsonb(t) FROM (SELECT {columns} FROM {} r WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4) t",t.name()),&[&tenant,&company,&after,&limit]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>();
    let next = if rows.len() as i64 == limit {
        rows.last().and_then(|r| r["id"].as_i64())
    } else {
        None
    };
    Ok(EvidencePage {
        fields: t.fields().to_vec(),
        rows,
        next,
    })
}
