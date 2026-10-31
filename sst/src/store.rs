//! Private owned SQL: the audited insert every SST write uses.
use bilibi_database::{Change, Error, Result};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

#[derive(Clone, Copy)]
pub(crate) enum Table {
    Registration,
    Mapping,
    Return,
    ReturnEvent,
    Adjustment,
    ReturnBox,
    ReturnSource,
    ReturnAccount,
}
impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Registration => "finance_sst_registrations",
            Self::Mapping => "finance_sst_mappings",
            Self::Return => "finance_sst_returns",
            Self::ReturnEvent => "finance_sst_return_events",
            Self::Adjustment => "finance_sst_return_adjustments",
            Self::ReturnBox => "finance_sst_return_boxes",
            Self::ReturnSource => "finance_sst_return_sources",
            Self::ReturnAccount => "finance_sst_return_accounts",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Registration => "finance.sst.registration",
            Self::Mapping => "finance.sst.mapping",
            Self::Return => "finance.sst.return",
            Self::ReturnEvent => "finance.sst.return_event",
            Self::Adjustment => "finance.sst.adjustment",
            Self::ReturnBox => "finance.sst.return_box",
            Self::ReturnSource => "finance.sst.return_source",
            Self::ReturnAccount => "finance.sst.return_account",
        }
    }
}

/// Who records a row: the tenant, the company, the signed-in actor and any
/// impersonator, never an argument.
pub(crate) fn identity(s: &Scope, company: i64) -> Value {
    json!({"tenant_id":s.tenant(),"company_id":company,"recorded_by_user_id":s.user(),"recorded_by_impersonator_id":s.impersonator().map(|p|p.user())})
}

/// Inserts `value` into `t` and returns the row as stored, numbered and
/// stamped by the database, with its audit `Change`.
pub(crate) async fn insert(
    c: &(impl GenericClient + Sync),
    t: Table,
    value: Value,
) -> Result<(i64, Change)> {
    let name = t.name();
    let r = c.query_one(&format!("INSERT INTO {name} SELECT (jsonb_populate_record(NULL::{name}, $1::jsonb || jsonb_build_object('id',nextval(pg_get_serial_sequence('{name}','id'))))).* RETURNING id,to_jsonb({name})"), &[&value]).await?;
    let id: i64 = r.get(0);
    if id <= 0 {
        return Err(Error::Invalid);
    }
    Ok((
        id,
        Change {
            kind: t.kind().into(),
            id,
            old: Value::Null,
            new: r.get(1),
        },
    ))
}

/// A `timestamp(6)` column as ISO 8601 text in UTC.
pub(crate) fn stamp(column: &str) -> String {
    format!("to_char({column},'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
}

pub(crate) fn date(text: &str) -> Result<jiff::civil::Date> {
    text.parse().map_err(|_| Error::Invalid)
}

/// A `numeric` column read as `::text`, exactly.
pub(crate) fn decimal(text: &str) -> Result<rust_decimal::Decimal> {
    rust_decimal::Decimal::from_str_exact(text).map_err(|_| Error::Invalid)
}

/// Merges `fields` into a row's `identity`.
pub(crate) fn row(mut identity: Value, fields: Value) -> Value {
    if let (Some(row), Value::Object(fields)) = (identity.as_object_mut(), fields) {
        row.extend(fields);
    }
    identity
}
