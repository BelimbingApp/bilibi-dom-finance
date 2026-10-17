//! Bilimbi.Finance.Payables.Matching: the three-way match. Each bill line
//! naming an order line is compared with what Payables' copy of the order
//! source says was ordered and received, less what this module already billed
//! against that line, within the company's tolerances. The result is the
//! figures compared and the exceptions found, which the facade records as a
//! match event. Callers authorize first.
use crate::*;
use rust_decimal::RoundingStrategy;
use std::collections::BTreeMap;

/// One per rule; the migration's check lists the same codes.
pub const MATCH_EXCEPTION_CODES: [&str; 8] = [
    "order_source_unavailable",
    "order_line_not_found",
    "order_line_superseded",
    "order_not_open",
    "currency_mismatch",
    "quantity_exceeds_ordered",
    "quantity_exceeds_received",
    "price_differs",
];
/// `pending` until first posted, `held` with unapproved exceptions,
/// `exception_approved` once approved, `matched` when it posted within
/// tolerance and `posted_with_exceptions` when it posted over approved ones.
pub const MATCH_STATUSES: [&str; 5] = [
    "pending",
    "held",
    "exception_approved",
    "matched",
    "posted_with_exceptions",
];
/// The order statuses a bill is matched against; any other order is not open
/// to billing.
const OPEN_ORDER_STATUSES: [&str; 4] = ["approved", "sent", "partially_received", "closed"];

/// Matches the bill's order lines: one result per bill line naming an order
/// line, each with the figures compared and its exceptions, empty when the
/// line matched. A bill naming one order line on several lines is matched
/// cumulatively: each counts the bill's earlier lines as already billed.
pub(crate) async fn evaluate(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    document: &Document,
    settings: &Settings,
    book_currency: &str,
    source: bool,
) -> Result<Vec<MatchLine>> {
    let lines: Vec<_> = document
        .lines
        .iter()
        .filter(|l| l.order_line_id.is_some())
        .collect();
    let ids: Vec<i64> = lines.iter().filter_map(|l| l.order_line_id).collect();
    let mut billed = billed_quantities(c, tenant, company, &ids, Some(document.id)).await?;
    let mut results = Vec::new();
    for line in lines {
        let (Some(id), Some(quantity)) = (line.order_line_id, line.quantity) else {
            continue;
        };
        let previously = billed.get(&id).copied().unwrap_or_default();
        let mut result = MatchLine {
            line_number: line.line_number,
            order_line_id: id,
            order_number: None,
            billed_quantity: quantity,
            billed_unit_price: (line.amount.amount() / quantity)
                .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero),
            previously_billed_quantity: previously,
            ordered_quantity: None,
            ordered_unit_price: None,
            received_quantity: None,
            quantity_tolerance: settings.quantity_tolerance.clone(),
            price_tolerance: settings.price_tolerance.clone(),
            exceptions: Vec::new(),
        };
        if !source {
            exception(
                &mut result,
                "order_source_unavailable",
                "no order source is installed to match against".into(),
            );
        } else if let Some(order) = orders::order_line(c, tenant, company, id).await? {
            result.order_number = order.order_number;
            result.ordered_quantity = Some(order.quantity);
            result.ordered_unit_price = Some(order.unit_price);
            result.received_quantity = Some(order.received_quantity);
            check_order(&mut result, &order);
            let same_currency = order.currency_code == book_currency;
            if !same_currency {
                exception(
                    &mut result,
                    "currency_mismatch",
                    format!(
                        "the order is in {}, the book in {book_currency}",
                        order.currency_code
                    ),
                );
            }
            check_quantities(&mut result, &order);
            // A currency mismatch already says the prices cannot be compared.
            if same_currency {
                check_price(&mut result, &order);
            }
        } else {
            exception(
                &mut result,
                "order_line_not_found",
                "the order source has no such order line".into(),
            );
        }
        *billed.entry(id).or_default() += quantity;
        results.push(result);
    }
    Ok(results)
}

fn exception(line: &mut MatchLine, code: &str, detail: String) {
    line.exceptions.push(MatchException {
        code: code.into(),
        detail,
    });
}
fn number(order: &orders::OrderLine) -> String {
    order
        .order_number
        .map(|n| n.to_string())
        .unwrap_or_default()
}
fn check_order(line: &mut MatchLine, order: &orders::OrderLine) {
    if order.superseded_by_amendment_id.is_some() {
        exception(
            line,
            "order_line_superseded",
            format!(
                "the order line was replaced by an amendment of order {}",
                number(order)
            ),
        );
    } else if !OPEN_ORDER_STATUSES.contains(&order.order_status.as_str()) {
        exception(
            line,
            "order_not_open",
            format!(
                "order {} is {}, not open to billing",
                number(order),
                order.order_status
            ),
        );
    }
}
fn check_quantities(line: &mut MatchLine, order: &orders::OrderLine) {
    let ordered = order.quantity;
    let allowance = allowance(&line.quantity_tolerance, ordered);
    let total = line.previously_billed_quantity + line.billed_quantity;
    let billable = order.received_quantity - line.previously_billed_quantity;
    if total > ordered + allowance {
        let detail = format!(
            "{} billed in all against {} ordered{}",
            text(total),
            text(ordered),
            within(allowance)
        );
        exception(line, "quantity_exceeds_ordered", detail);
    }
    if line.billed_quantity > billable + allowance {
        let detail = format!(
            "{} billed against {} received and not yet billed{}",
            text(line.billed_quantity),
            text(billable),
            within(allowance)
        );
        exception(line, "quantity_exceeds_received", detail);
    }
}
fn check_price(line: &mut MatchLine, order: &orders::OrderLine) {
    let ordered = order.unit_price;
    let allowance = allowance(&line.price_tolerance, ordered);
    if (line.billed_unit_price - ordered).abs() > allowance {
        let detail = format!(
            "{} per unit billed against {} ordered{}",
            text(line.billed_unit_price),
            text(ordered),
            within(allowance)
        );
        exception(line, "price_differs", detail);
    }
}
/// A tolerance is a percentage of the ordered figure or an amount in its
/// unit; none means an exact match.
fn allowance(tolerance: &Option<Tolerance>, base: Decimal) -> Decimal {
    match tolerance {
        None => Decimal::ZERO,
        Some(Tolerance {
            kind: ToleranceKind::Amount,
            value,
        }) => *value,
        Some(Tolerance {
            kind: ToleranceKind::Percent,
            value,
        }) => base * value / Decimal::ONE_HUNDRED,
    }
}
fn within(allowance: Decimal) -> String {
    if allowance.is_zero() {
        String::new()
    } else {
        format!(", within {}", text(allowance))
    }
}
fn text(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Whether an approval covers a fresh match: `approved` is the match lines of
/// the hold that was approved, `lines` the fresh results. Each exception must
/// be one the approved line had, with figures no worse than the approver saw.
pub(crate) fn covered(approved: &[MatchLine], lines: &[MatchLine]) -> bool {
    lines.iter().all(|line| {
        match approved.iter().find(|seen| {
            seen.line_number == line.line_number && seen.order_line_id == line.order_line_id
        }) {
            None => line.exceptions.is_empty(),
            Some(seen) => line.exceptions.iter().all(|x| {
                seen.exceptions.iter().any(|s| s.code == x.code)
                    && match x.code.as_str() {
                        "quantity_exceeds_ordered" => overrun(line) <= overrun(seen),
                        "quantity_exceeds_received" => shortfall(line) <= shortfall(seen),
                        "price_differs" => price_gap(line) <= price_gap(seen),
                        _ => true,
                    }
            }),
        }
    })
}
fn overrun(line: &MatchLine) -> Decimal {
    line.previously_billed_quantity + line.billed_quantity
        - line.ordered_quantity.unwrap_or_default()
}
fn shortfall(line: &MatchLine) -> Decimal {
    line.previously_billed_quantity + line.billed_quantity
        - line.received_quantity.unwrap_or_default()
}
fn price_gap(line: &MatchLine) -> Decimal {
    (line.billed_unit_price - line.ordered_unit_price.unwrap_or_default()).abs()
}

/// The quantity billed against each order line, by ID: the quantities of
/// posted bill lines naming it, less those of posted debit note lines, in the
/// company. `except` leaves out the document being matched.
pub(crate) async fn billed_quantities(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    ids: &[i64],
    except: Option<i64>,
) -> Result<BTreeMap<i64, Decimal>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(c.query(
        "SELECT l.order_line_id,sum(CASE WHEN d.document_type='bill' THEN l.quantity ELSE -l.quantity END) FROM finance_payables_document_lines l JOIN finance_payables_documents d ON d.id=l.document_id AND d.tenant_id=l.tenant_id WHERE l.tenant_id=$1 AND l.company_id=$2 AND l.order_line_id=ANY($3) AND d.status='posted' AND ($4::bigint IS NULL OR l.document_id<>$4) GROUP BY l.order_line_id",
        &[&tenant, &company, &ids, &except],
    )
    .await?
    .iter()
    .map(|r| (r.get(0), r.get::<_, Decimal>(1).normalize()))
    .collect())
}

/// The quantities a debit note's order lines take back, checked against the
/// corrected bill: each order line is one the bill names, and the quantity
/// with what earlier posted debit notes took back fits what the bill billed.
/// The refusal names the first line that does not.
pub(crate) async fn within_billed(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    bill: &Document,
    lines: &[(usize, Option<i64>, Option<Decimal>)],
    except: Option<i64>,
) -> Result<BTreeMap<i64, Decimal>> {
    let mut billed: BTreeMap<i64, Decimal> = BTreeMap::new();
    for l in &bill.lines {
        if let (Some(id), Some(q)) = (l.order_line_id, l.quantity) {
            *billed.entry(id).or_default() += q;
        }
    }
    let mut taking: Vec<(i64, usize, Decimal)> = Vec::new();
    for (number, id, quantity) in lines {
        if let (Some(id), Some(q)) = (id, quantity) {
            match taking.iter_mut().find(|(t, _, _)| t == id) {
                Some((_, _, sum)) => *sum += *q,
                None => taking.push((*id, *number, *q)),
            }
        }
    }
    if taking.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ids: Vec<i64> = billed.keys().copied().collect();
    let taken: BTreeMap<i64, Decimal> = c
        .query(
            "SELECT l.order_line_id,sum(l.quantity) FROM finance_payables_document_lines l JOIN finance_payables_documents d ON d.id=l.document_id AND d.tenant_id=l.tenant_id WHERE l.tenant_id=$1 AND l.company_id=$2 AND l.order_line_id=ANY($3) AND d.status='posted' AND d.corrects_document_id=$4 AND ($5::bigint IS NULL OR l.document_id<>$5) GROUP BY l.order_line_id",
            &[&tenant, &company, &ids, &bill.id, &except],
        )
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    let mut capacity = BTreeMap::new();
    for (id, first, quantity) in taking {
        let refusal = |field, message: &str| {
            PayablesError::Invalid(vec![FieldError {
                line: Some(first),
                ..FieldError::new(field, message)
            }])
        };
        let Some(on_bill) = billed.get(&id) else {
            return Err(refusal(
                "order_line_id",
                "is not an order line of the corrected bill",
            ));
        };
        if quantity + taken.get(&id).copied().unwrap_or_default() > *on_bill {
            return Err(refusal(
                "quantity",
                "exceeds the quantity billed on the corrected bill",
            ));
        }
        capacity.insert(
            id,
            (*on_bill - taken.get(&id).copied().unwrap_or_default()).normalize(),
        );
    }
    Ok(capacity)
}

/// Nil unless a bill names an order line; else what its latest match event
/// says, or pending before any.
pub(crate) fn status(document: &Document) -> Option<String> {
    if document.document_type != "bill"
        || document.status == "voided"
        || document.lines.iter().all(|l| l.order_line_id.is_none())
    {
        return None;
    }
    Some(
        match document.match_events.last() {
            None => "pending",
            Some(e) if e.event == "posted" => {
                if e.lines.iter().any(|l| !l.exceptions.is_empty()) {
                    "posted_with_exceptions"
                } else {
                    "matched"
                }
            }
            Some(e) if e.event == "held" => "held",
            Some(_) => "exception_approved",
        }
        .into(),
    )
}
