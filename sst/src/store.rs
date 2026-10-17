//! Private owned SQL: the audited insert every SST write uses.
use bilibi_database::{Change, Error, Result};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

#[derive(Clone, Copy)]
pub(crate) enum Table {
    Registration,
    Mapping,
}
impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Registration => "finance_sst_registrations",
            Self::Mapping => "finance_sst_mappings",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Registration => "finance.sst.registration",
            Self::Mapping => "finance.sst.mapping",
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
pub(crate) async fn insert(c: &impl GenericClient, t: Table, value: Value) -> Result<(i64, Change)> {
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
