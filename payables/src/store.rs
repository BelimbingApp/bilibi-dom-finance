//! Private SQL for Bilimbi.Finance.Payables records. Never reads sibling tables.
use crate::*;
use bilibi_database::{Change, Result as DbResult};
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

#[derive(Clone, Copy)]
pub(crate) enum Table {
    Settings,
    TaxCodes,
    SettingsChanges,
    TaxCodeChanges,
    Documents,
    Lines,
    TaxLines,
    Allocations,
    Requests,
    RequestAllocations,
    RequestEvents,
}
impl Table {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Settings => "finance_payables_settings",
            Self::TaxCodes => "finance_payables_tax_codes",
            Self::SettingsChanges => "finance_payables_settings_changes",
            Self::TaxCodeChanges => "finance_payables_tax_code_changes",
            Self::Documents => "finance_payables_documents",
            Self::Lines => "finance_payables_document_lines",
            Self::TaxLines => "finance_payables_document_tax_lines",
            Self::Allocations => "finance_payables_payment_allocations",
            Self::Requests => "finance_payables_payment_requests",
            Self::RequestAllocations => "finance_payables_payment_request_allocations",
            Self::RequestEvents => "finance_payables_payment_request_events",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Self::Settings => "finance.payables.settings",
            Self::TaxCodes => "finance.payables.tax_code",
            Self::SettingsChanges => "finance.payables.settings_change",
            Self::TaxCodeChanges => "finance.payables.tax_code_change",
            Self::Documents => "finance.payables.document",
            Self::Lines => "finance.payables.document_line",
            Self::TaxLines => "finance.payables.document_tax_line",
            Self::Allocations => "finance.payables.allocation",
            Self::Requests => "finance.payables.payment_request",
            Self::RequestAllocations => "finance.payables.payment_request_allocation",
            Self::RequestEvents => "finance.payables.payment_request_event",
        }
    }
}
/// All identifiers come from the private enum. Populate only this module's
/// rows; every inserted row is returned verbatim for mutation capture.
pub(crate) async fn insert(
    c: &impl GenericClient,
    table: Table,
    value: Value,
) -> DbResult<(Value, Change)> {
    let name = table.name();
    let row=c.query_one(&format!("INSERT INTO {name} SELECT (jsonb_populate_record(NULL::{name}, $1::jsonb || jsonb_build_object('id',nextval(pg_get_serial_sequence('{name}','id')), 'inserted_at',timezone('UTC',clock_timestamp()), 'updated_at',timezone('UTC',clock_timestamp())))).* RETURNING to_jsonb({name})"),&[&value]).await?;
    let value: Value = row.get(0);
    let id = value["id"].as_i64().ok_or(Error::Invalid)?;
    Ok((
        value.clone(),
        Change {
            kind: table.kind().into(),
            id,
            old: Value::Null,
            new: value,
        },
    ))
}
pub(crate) fn update_change(table: Table, id: i64, before: Value, after: Value) -> Change {
    Change {
        kind: table.kind().into(),
        id,
        old: before,
        new: after,
    }
}
pub(crate) async fn raw(
    c: &impl GenericClient,
    t: Table,
    tenant: i64,
    company: i64,
    id: i64,
) -> DbResult<Value> {
    let n = t.name();
    Ok(c.query_one(
        &format!("SELECT to_jsonb(t) FROM {n} t WHERE tenant_id=$1 AND company_id=$2 AND id=$3"),
        &[&tenant, &company, &id],
    )
    .await?
    .get(0))
}
pub(crate) async fn settings(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> DbResult<Option<(i64, Settings)>> {
    let r=c.query_opt("SELECT id,payable_account_id,quantity_tolerance_kind,quantity_tolerance,price_tolerance_kind,price_tolerance FROM finance_payables_settings WHERE tenant_id=$1 AND company_id=$2",&[&tenant,&company]).await?;
    Ok(r.map(|r| {
        (
            r.get(0),
            Settings {
                payable_account_id: r.get(1),
                quantity_tolerance: tolerance(&r, 2, 3),
                price_tolerance: tolerance(&r, 4, 5),
            },
        )
    }))
}
fn tolerance(r: &Row, k: usize, v: usize) -> Option<Tolerance> {
    r.get::<_, Option<String>>(k).map(|kind| Tolerance {
        kind: if kind == "percent" {
            ToleranceKind::Percent
        } else {
            ToleranceKind::Amount
        },
        value: r.get(v),
    })
}
pub(crate) fn settings_json(s: &Settings) -> Value {
    json!({"payable_account_id":s.payable_account_id,
        "quantity_tolerance_kind":s.quantity_tolerance.as_ref().map(|t|t.kind),"quantity_tolerance":s.quantity_tolerance.as_ref().map(|t|t.value),
        "price_tolerance_kind":s.price_tolerance.as_ref().map(|t|t.kind),"price_tolerance":s.price_tolerance.as_ref().map(|t|t.value)})
}
pub(crate) async fn taxes(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> DbResult<Vec<TaxCode>> {
    Ok(c.query("SELECT id,code,name,rate,account_id,active FROM finance_payables_tax_codes WHERE tenant_id=$1 AND company_id=$2 ORDER BY code",&[&tenant,&company]).await?.iter().map(|r| TaxCode {id:r.get(0),code:r.get(1),name:r.get(2),rate:r.get(3),account_id:r.get(4),active:r.get(5)}).collect())
}
pub(crate) fn money(d: Decimal) -> DbResult<Money> {
    Money::new(
        d,
        bilibi_finance_ledger::MinorUnits::new(4).ok_or(Error::Invalid)?,
    )
    .map_err(|_| Error::Invalid)
}
fn text(r: &Value, k: &str) -> DbResult<String> {
    r[k].as_str().map(str::to_owned).ok_or(Error::Invalid)
}
fn opt_text(r: &Value, k: &str) -> Option<String> {
    r[k].as_str().map(str::to_owned)
}
fn int(r: &Value, k: &str) -> DbResult<i64> {
    r[k].as_i64().ok_or(Error::Invalid)
}
fn opt_int(r: &Value, k: &str) -> Option<i64> {
    r[k].as_i64()
}
fn amount(r: &Value, k: &str) -> DbResult<Money> {
    // Keep PostgreSQL numeric exact even with serde_json/arbitrary_precision;
    // Decimal's default visitor does not accept its private number map.
    let d = Decimal::from_str_exact(&r[k].to_string()).map_err(|_| Error::Invalid)?;
    money(d)
}
pub(crate) async fn document(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    id: i64,
) -> DbResult<Option<Document>> {
    let Some(row)=c.query_opt("SELECT to_jsonb(d) FROM finance_payables_documents d WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id]).await? else {return Ok(None)};
    let v: Value = row.get(0);
    let mut lines = Vec::new();
    for r in c.query("SELECT id,line_number,description,account_id,amount,tax_code_id,external_order_line_id,order_line_id,quantity FROM finance_payables_document_lines WHERE tenant_id=$1 AND company_id=$2 AND document_id=$3 ORDER BY line_number",&[&tenant,&company,&id]).await? {
        lines.push(DocumentLine {id:r.get(0),line_number:r.get(1),description:r.get(2),account_id:r.get(3),amount:money(r.get(4))?,tax_code_id:r.get(5),external_order_line_id:r.get(6),order_line_id:r.get(7),quantity:r.get(8)});
    }
    let mut tax_lines = Vec::new();
    for r in c.query("SELECT id,line_number,tax_code_id,tax_code,rate,account_id,taxable_amount,tax_amount FROM finance_payables_document_tax_lines WHERE tenant_id=$1 AND company_id=$2 AND document_id=$3 ORDER BY line_number",&[&tenant,&company,&id]).await? {
        tax_lines.push(TaxLine {id:r.get(0),line_number:r.get(1),tax_code_id:r.get(2),tax_code:r.get(3),rate:r.get(4),account_id:r.get(5),taxable_amount:money(r.get(6))?,tax_amount:money(r.get(7))?});
    }
    let debit_note_ids=c.query("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND corrects_document_id=$3 ORDER BY id",&[&tenant,&company,&id]).await?.iter().map(|r|r.get(0)).collect();
    let status = text(&v, "status")?;
    let kind = text(&v, "document_type")?;
    let outstanding = if kind == "bill" && status == "posted" {
        Some(outstanding(c, tenant, company, id).await?)
    } else {
        None
    };
    Ok(Some(Document {
        id,
        document_type: kind,
        status,
        document_number: opt_int(&v, "document_number"),
        corrects_document_id: opt_int(&v, "corrects_document_id"),
        party: Party {
            key: text(&v, "party_key")?,
            name: text(&v, "party_name")?,
            tax_id: opt_text(&v, "party_tax_id"),
            registration_id: opt_text(&v, "party_registration_id"),
            address: opt_text(&v, "party_address"),
        },
        party_document_number: opt_text(&v, "party_document_number"),
        document_date: text(&v, "document_date")?
            .parse()
            .map_err(|_| Error::Invalid)?,
        due_date: text(&v, "due_date")?.parse().map_err(|_| Error::Invalid)?,
        currency_code: text(&v, "currency_code")?,
        description: opt_text(&v, "description"),
        total_net: amount(&v, "total_net")?,
        total_tax: amount(&v, "total_tax")?,
        total: amount(&v, "total")?,
        created_by_user_id: int(&v, "created_by_user_id")?,
        created_by_impersonator_id: opt_int(&v, "created_by_impersonator_id"),
        created_at: text(&v, "created_at")?,
        approved_by_user_id: opt_int(&v, "approved_by_user_id"),
        approved_by_impersonator_id: opt_int(&v, "approved_by_impersonator_id"),
        approved_at: opt_text(&v, "approved_at"),
        posted_by_user_id: opt_int(&v, "posted_by_user_id"),
        posted_by_impersonator_id: opt_int(&v, "posted_by_impersonator_id"),
        posted_at: opt_text(&v, "posted_at"),
        voided_by_user_id: opt_int(&v, "voided_by_user_id"),
        voided_by_impersonator_id: opt_int(&v, "voided_by_impersonator_id"),
        voided_at: opt_text(&v, "voided_at"),
        void_reason: opt_text(&v, "void_reason"),
        payable_account_id: opt_int(&v, "payable_account_id"),
        entry_id: opt_int(&v, "entry_id"),
        entry_number: opt_int(&v, "entry_number"),
        lines,
        tax_lines,
        debit_note_ids,
        outstanding,
    }))
}
/// Current balance uses the same temporal calculation as reports and writes.
pub(crate) async fn outstanding(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    bill: i64,
) -> DbResult<Money> {
    crate::balances::owed(
        c,
        tenant,
        company,
        bill,
        jiff::civil::date(9999, 12, 31),
        i64::MAX,
    )
    .await
}
