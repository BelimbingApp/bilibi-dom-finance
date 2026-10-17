//! Payables' own copy of the order source, replacing Bilimbi's runtime
//! `OrderSource`. Supply Chain's documented domain events feed it; their
//! publisher is named only by event name, never by crate, descriptor edge,
//! port or SQL, so Finance composes and boots without Supply Chain and then
//! simply receives nothing. Payloads parse into this module's own types.
use crate::store::{self, Table};
use crate::*;
use bilibi_authz::SystemScope;
use bilibi_database::{Change, Result as DbResult};
use bilibi_events::{Event, Events, On, Subscribe, Subscriber, SubscriberFuture, SUBSCRIBERS};
use bilibi_module_registry::{Contribution, Provenance};
use serde_json::{json, Value};
use tokio_postgres::Transaction;

// Supply Chain Receiving's event contract, version 1 (its receiving/README.md).
const GOODS_RECEIVED: &str = "supply_chain/receiving.goods_received";
const RECEIPT_REVERSED: &str = "supply_chain/receiving.receipt_reversed";
/// Every event the copy reads. The order source is available only while all
/// of them are mounted event types: a line cannot name an order line before,
/// and a bill naming one is held as `order_source_unavailable`.
// TODO(domain events): add Purchasing's order_status_changed, order_approved
// and order_amended (unit price, currency, status, supersession) once the
// pinned Supply Chain declares them; until then a copied line has no order
// facts and matches as `order_line_not_found`.
const SOURCE_EVENTS: [&str; 2] = [GOODS_RECEIVED, RECEIPT_REVERSED];

pub(crate) fn available(events: &Events) -> bool {
    SOURCE_EVENTS
        .iter()
        .all(|name| events.event_types().any(|mounted| mounted == *name))
}

inventory::submit! { Contribution::new(SUBSCRIBERS, "finance/payables.goods_receipts", Provenance { module: MODULE, source: file!(), line: line!() }, &Subscriber {
    events: &[On { event: GOODS_RECEIVED, versions: &[1] }, On { event: RECEIPT_REVERSED, versions: &[1] }],
    handler: &GoodsReceipts,
}) }

#[derive(Deserialize)]
struct ReceiptPosted {
    company_id: i64,
    posting_id: i64,
    posted_at: String,
    receipt_id: i64,
    receipt_number: i64,
    kind: String,
    order_id: i64,
    order_number: Option<i64>,
    line: ReceiptLine,
}
#[derive(Deserialize)]
struct ReceiptLine {
    order_line_id: i64,
    order_line_number: Option<i32>,
    description: Option<String>,
    unit: Option<String>,
    ordered_quantity: Option<String>,
    received_quantity: String,
    rejected_quantity: String,
}

/// One goods receipt line, or its reversal, posted against an order line.
/// A receipt adds its accepted quantity to what was received and a reversal
/// takes it off; the event key makes a redelivery write nothing.
struct GoodsReceipts;
impl Subscribe for GoodsReceipts {
    fn handle<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        scope: &'a SystemScope,
        event: &'a Event,
    ) -> SubscriberFuture<'a> {
        Box::pin(async move {
            let posted: ReceiptPosted =
                serde_json::from_value(event.payload.clone()).map_err(|_| Error::Invalid)?;
            let kind = match (event.name.as_str(), posted.kind.as_str()) {
                (GOODS_RECEIVED, "receipt") => "receipt",
                (RECEIPT_REVERSED, "reversal") => "reversal",
                _ => return Err(Error::Invalid),
            };
            let tenant = scope.tenant();
            let company = posted.company_id;
            let line = &posted.line;
            if tx
                .query_opt(
                    "SELECT 1 FROM finance_payables_order_receipts WHERE tenant_id=$1 AND event_key=$2",
                    &[&tenant, &event.key],
                )
                .await?
                .is_some()
            {
                return Ok(Vec::new());
            }
            let mut changes = Vec::new();
            let facts = json!({"order_id": posted.order_id, "order_number": posted.order_number,
                "line_number": line.order_line_number, "description": line.description, "unit": line.unit,
                "quantity": line.ordered_quantity.as_deref().map(quantity).transpose()?});
            keep_line(tx, tenant, company, line.order_line_id, facts, &mut changes).await?;
            let row = json!({"tenant_id": tenant, "company_id": company, "order_line_id": line.order_line_id,
                "event_key": event.key, "kind": kind, "posting_id": posted.posting_id,
                "receipt_id": posted.receipt_id, "receipt_number": posted.receipt_number,
                "posted_at": posted.posted_at.trim_end_matches('Z'),
                "received_quantity": quantity(&line.received_quantity)?,
                "rejected_quantity": quantity(&line.rejected_quantity)?});
            changes.push(store::insert(tx, Table::OrderReceipts, row).await?.1);
            Ok(changes)
        })
    }
}

/// A source quantity: decimal text with at most four places, as the
/// `numeric(20,4)` columns hold it.
fn quantity(text: &str) -> DbResult<Decimal> {
    let value = Decimal::from_str_exact(text).map_err(|_| Error::Invalid)?;
    if value.scale() > 4 || value.is_sign_negative() || value.abs() >= Decimal::from(10_i64.pow(16))
    {
        return Err(Error::Invalid);
    }
    Ok(value.normalize())
}

/// Creates the copy of an order line, or fills the facts it still lacks.
/// A receipt's snapshot never overwrites what the copy already holds.
async fn keep_line(
    tx: &Transaction<'_>,
    tenant: i64,
    company: i64,
    order_line: i64,
    facts: Value,
    changes: &mut Vec<Change>,
) -> DbResult<()> {
    let existing = tx
        .query_opt(
            "SELECT id,to_jsonb(l) FROM finance_payables_order_lines l WHERE tenant_id=$1 AND company_id=$2 AND order_line_id=$3",
            &[&tenant, &company, &order_line],
        )
        .await?;
    let Some(existing) = existing else {
        let mut row = facts;
        row["tenant_id"] = json!(tenant);
        row["company_id"] = json!(company);
        row["order_line_id"] = json!(order_line);
        changes.push(store::insert(tx, Table::OrderLines, row).await?.1);
        return Ok(());
    };
    let (id, before): (i64, Value) = (existing.get(0), existing.get(1));
    let after: Value = tx.query_one(
        "UPDATE finance_payables_order_lines SET order_number=COALESCE(order_number,($4::jsonb->>'order_number')::bigint), line_number=COALESCE(line_number,($4::jsonb->>'line_number')::integer), description=COALESCE(description,$4::jsonb->>'description'), unit=COALESCE(unit,$4::jsonb->>'unit'), quantity=COALESCE(quantity,($4::jsonb->>'quantity')::numeric), updated_at=timezone('UTC',clock_timestamp()) WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(finance_payables_order_lines)",
        &[&tenant, &company, &id, &facts],
    ).await?.get(0);
    let mut compared = (before.clone(), after.clone());
    for v in [&mut compared.0, &mut compared.1] {
        v["updated_at"] = Value::Null;
    }
    if compared.0 != compared.1 {
        changes.push(store::update_change(Table::OrderLines, id, before, after));
    }
    Ok(())
}

/// What the copy says of one order line now: its order facts once the order
/// source has sent them, and the quantity received against it net of
/// reversals. None when the copy has no such line with order facts.
pub(crate) struct OrderLine {
    pub order_number: Option<i64>,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub currency_code: String,
    pub order_status: String,
    pub superseded_by_amendment_id: Option<i64>,
    pub received_quantity: Decimal,
}
pub(crate) async fn order_line(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    order_line: i64,
) -> DbResult<Option<OrderLine>> {
    let row = c.query_opt(
        "SELECT l.order_number,l.quantity,l.unit_price,l.currency_code,l.order_status,l.superseded_by_amendment_id,\
         COALESCE((SELECT sum(CASE WHEN r.kind='receipt' THEN r.received_quantity ELSE -r.received_quantity END) FROM finance_payables_order_receipts r WHERE r.company_id=l.company_id AND r.order_line_id=l.order_line_id),0) \
         FROM finance_payables_order_lines l WHERE l.tenant_id=$1 AND l.company_id=$2 AND l.order_line_id=$3 AND l.order_event_key IS NOT NULL",
        &[&tenant, &company, &order_line],
    ).await?;
    Ok(row.map(|r| OrderLine {
        order_number: r.get(0),
        quantity: r.get(1),
        unit_price: r.get(2),
        currency_code: r.get(3),
        order_status: r.get(4),
        superseded_by_amendment_id: r.get(5),
        received_quantity: r.get::<_, Decimal>(6).max(Decimal::ZERO),
    }))
}
/// The order lines among `ids` the copy knows in the company, from any event.
pub(crate) async fn known(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    ids: &[i64],
) -> DbResult<Vec<i64>> {
    Ok(c.query(
        "SELECT order_line_id FROM finance_payables_order_lines WHERE tenant_id=$1 AND company_id=$2 AND order_line_id=ANY($3)",
        &[&tenant, &company, &ids],
    )
    .await?
    .iter()
    .map(|r| r.get(0))
    .collect())
}
