//! Bilimbi.Finance.Sst.Figures: a return's figures, the document tax of the
//! period read through the ledger, receivables and payables facades at one
//! ledger entry number, routed to boxes by the tax code mappings in force
//! through one mapping number (and, from NS2, a return's standing
//! adjustments); and
//! how each tax account's ledger postings compare with the documents behind
//! them.
//!
//! Routing, by each row's tax code mapping:
//!
//! - no mapping: listed as unmapped, which stops the return finalising;
//! - mapped as not reported (no box): listed, counted nowhere;
//! - mapped to the other tax's form: left to that return;
//! - mapped to a box of this form: an invoice or bill adds its value and tax;
//!   a credit note on an invoice of the same period, or a debit note,
//!   subtracts them; a credit note on an earlier period's invoice deducts its
//!   tax in item 13(a), where a credit note for tax already paid goes.
//!
//! Service tax due when payment is received reads receivables' tax share of
//! payments instead of its invoices: `received` and `unpaid` add to the
//! mapped box, and `credited_after_due` deducts in item 13(a).
use crate::form::{self, BoxFigure, BoxKind, FormBox};
use crate::registrations::Registration;
use crate::*;
use bilibi_finance_ledger::TotalsQuery;
use bilibi_finance_payables::TaxQuery;
use bilibi_finance_receivables::{ReceivedTaxOptions, TaxOptions};
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};

/// Section 11(2) of the Service Tax Act 2018: service tax not paid within
/// twelve months of the invoice is due the day after.
const UNPAID_MONTHS: i32 = 12;

/// The return a reading is for: one tax over one taxable period.
pub(crate) struct Period {
    pub tax_type: TaxType,
    pub period_start: Date,
    pub period_end: Date,
}

/// What one group of documents counted: per source, document kind, measure,
/// tax code, rate and tax account, the box it went to (`None` when the code
/// is mapped as not reported) and its signed contribution to that box, kept
/// at the columns' four places so stored rows add up to their boxes exactly.
/// `measure` is `documents` (by document date), `received` (the tax share of
/// payments received, net of reversals), `unpaid` (the share an invoice still
/// owed when the unpaid-period rule made it due) or `credited_after_due`
/// (credit notes on tax already due); `prior_period` marks a credit note on
/// an invoice of an earlier period.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceRow {
    pub line_number: i32,
    pub source: TaxSource,
    pub document_kind: String,
    pub measure: String,
    pub prior_period: bool,
    pub tax_code_id: i64,
    pub tax_code: String,
    pub rate: Decimal,
    pub account_id: i64,
    pub document_count: i64,
    #[serde(rename = "box")]
    pub box_key: Option<String>,
    pub mapping_number: i64,
    pub taxable_amount: Decimal,
    pub tax_amount: Decimal,
    pub rate_mismatch: bool,
}

/// A tax code with tax in the period and no mapping.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnmappedRow {
    pub source: TaxSource,
    pub document_kind: String,
    pub tax_code_id: i64,
    pub tax_code: String,
    pub rate: Decimal,
}

/// One tax account's ledger debits and credits over the period against what
/// the documents posted to it; `difference` is ledger credits less debits
/// less the documents' credits less debits, shown and never hidden.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AccountReconciliation {
    pub line_number: i32,
    pub account_id: i64,
    pub account_code: String,
    pub account_name: String,
    pub ledger_debits: Decimal,
    pub ledger_credits: Decimal,
    pub document_debits: Decimal,
    pub document_credits: Decimal,
    pub difference: Decimal,
}

/// A return's figures: `boxes` in the form's order, `tax_payable` (item
/// 14), every counted source row, the `unmapped` rows and `rate_mismatches`
/// that keep a return from finalising, and the ledger reconciliation, read
/// at `through_entry_number` and `through_mapping_number`.
#[derive(Clone, Debug, PartialEq)]
pub struct Figures {
    pub tax_type: TaxType,
    pub period_start: Date,
    pub period_end: Date,
    pub service_tax_basis: Option<String>,
    pub currency_code: String,
    pub minor_units: u8,
    pub through_entry_number: i64,
    pub through_mapping_number: i64,
    pub boxes: Vec<BoxFigure>,
    pub tax_payable: Decimal,
    pub sources: Vec<SourceRow>,
    pub unmapped: Vec<UnmappedRow>,
    pub rate_mismatches: Vec<SourceRow>,
    pub accounts: Vec<AccountReconciliation>,
}

/// A source row in one shape before routing: what a document group carried,
/// with `sign` 1 when it adds to its box and -1 when it deducts.
struct Row {
    source: TaxSource,
    document_kind: String,
    measure: String,
    prior_period: bool,
    tax_code_id: i64,
    tax_code: String,
    rate: Decimal,
    account_id: i64,
    document_count: i64,
    carried_taxable: Decimal,
    carried_tax: Decimal,
    sign: i8,
}
impl Row {
    fn signed(&self, amount: Decimal) -> Decimal {
        if self.sign < 0 {
            -amount
        } else {
            amount
        }
    }
    /// Whether the document credits its tax account: receivables' invoices
    /// credit their tax, payables' debit notes theirs.
    fn credits_account(&self) -> bool {
        match self.source {
            TaxSource::Receivables => self.document_kind == "invoice",
            TaxSource::Payables => self.document_kind == "debit_note",
        }
    }
    fn counted(&self, mapping: &Mapping, box_key: Option<&str>, taxable: Decimal, tax: Decimal) -> SourceRow {
        SourceRow {
            line_number: 0,
            source: self.source,
            document_kind: self.document_kind.clone(),
            measure: self.measure.clone(),
            prior_period: self.prior_period,
            tax_code_id: self.tax_code_id,
            tax_code: self.tax_code.clone(),
            rate: self.rate,
            account_id: self.account_id,
            document_count: self.document_count,
            box_key: box_key.map(str::to_owned),
            mapping_number: mapping.mapping_number,
            taxable_amount: form::round(taxable, 4),
            tax_amount: form::round(tax, 4),
            rate_mismatch: false,
        }
    }
}

enum Routed {
    Counted(SourceRow),
    Unmapped(UnmappedRow),
    Elsewhere,
}

type Mappings = HashMap<(TaxSource, i64), Mapping>;

fn service(row: &Row, mappings: &Mappings) -> bool {
    mappings
        .get(&(row.source, row.tax_code_id))
        .and_then(|m| m.box_key.as_deref())
        .and_then(form::mapped_tax_type)
        == Some(TaxType::ServiceTax)
}

fn route(row: &Row, mappings: &Mappings, tax_type: TaxType) -> Routed {
    let Some(mapping) = mappings.get(&(row.source, row.tax_code_id)) else {
        return Routed::Unmapped(UnmappedRow {
            source: row.source,
            document_kind: row.document_kind.clone(),
            tax_code_id: row.tax_code_id,
            tax_code: row.tax_code.clone(),
            rate: row.rate,
        });
    };
    let Some(key) = mapping.box_key.as_deref() else {
        return Routed::Counted(row.counted(
            mapping,
            None,
            row.signed(row.carried_taxable),
            row.signed(row.carried_tax),
        ));
    };
    match form::find(tax_type, key) {
        Some(found) if form::mapped_tax_type(key) == Some(tax_type) => {
            Routed::Counted(to_box(row, mapping, found))
        }
        _ => Routed::Elsewhere,
    }
}

/// A deduction of tax already reported goes to item 13(a); a value-only box
/// nets its credit notes in place, since it holds no tax to deduct.
fn to_box(row: &Row, mapping: &Mapping, found: &FormBox) -> SourceRow {
    let mut counted = if row.sign < 0 && row.prior_period && found.kind != BoxKind::Value {
        row.counted(
            mapping,
            Some(form::CREDIT_NOTE_BOX),
            Decimal::ZERO,
            row.carried_tax,
        )
    } else {
        row.counted(
            mapping,
            Some(found.key),
            row.signed(row.carried_taxable),
            row.signed(row.carried_tax),
        )
    };
    counted.rate_mismatch = match found.kind {
        BoxKind::Rated => found.rate() != Some(row.rate),
        BoxKind::Value => !row.carried_tax.is_zero(),
        _ => false,
    };
    counted
}

/// Each box's summed contributions. NS2 adds a return's standing
/// adjustments here, each to its box.
fn sums(sources: &[SourceRow]) -> HashMap<String, (Decimal, Decimal)> {
    let mut sums: HashMap<String, (Decimal, Decimal)> = HashMap::new();
    let contributions = sources
        .iter()
        .filter_map(|r| Some((r.box_key.clone()?, r.taxable_amount, r.tax_amount)));
    for (key, taxable, tax) in contributions {
        let sum = sums.entry(key).or_default();
        sum.0 += taxable;
        sum.1 += tax;
    }
    sums
}

/// Whether a row's code reports on this form (or has no mapping yet), so its
/// tax account belongs in this return's reconciliation.
fn form_code(row: &Row, mappings: &Mappings, tax_type: TaxType) -> bool {
    match mappings.get(&(row.source, row.tax_code_id)) {
        None => true,
        Some(Mapping { box_key: None, .. }) => false,
        Some(Mapping {
            box_key: Some(key), ..
        }) => form::mapped_tax_type(key) == Some(tax_type),
    }
}

/// Each tax account a code of this form (or an unmapped code) posted to in
/// the period, its debits and credits over the period against what every
/// document posted to it. Only the period's own rows name the accounts, so a
/// finalised return's reconciliation reproduces whatever tax codes say
/// later.
fn reconcile(
    accrual: &[Row],
    tax_type: TaxType,
    mappings: &Mappings,
    ledger: &HashMap<i64, (Decimal, Decimal)>,
    accounts: &HashMap<i64, (String, String)>,
) -> Vec<AccountReconciliation> {
    let mut seen = HashSet::new();
    let mut rows: Vec<AccountReconciliation> = accrual
        .iter()
        .filter(|r| form_code(r, mappings, tax_type) && seen.insert(r.account_id))
        .map(|r| {
            let id = r.account_id;
            let (debits, credits) = accrual.iter().filter(|d| d.account_id == id).fold(
                (Decimal::ZERO, Decimal::ZERO),
                |(debits, credits), d| {
                    if d.credits_account() {
                        (debits, credits + d.carried_tax)
                    } else {
                        (debits + d.carried_tax, credits)
                    }
                },
            );
            let (code, name) = accounts
                .get(&id)
                .cloned()
                .unwrap_or_else(|| ("?".into(), format!("Account {id}")));
            let (ledger_debits, ledger_credits) = ledger.get(&id).copied().unwrap_or_default();
            AccountReconciliation {
                line_number: 0,
                account_id: id,
                account_code: code,
                account_name: name,
                ledger_debits,
                ledger_credits,
                document_debits: debits,
                document_credits: credits,
                difference: ledger_credits - ledger_debits - (credits - debits),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.account_code.cmp(&b.account_code));
    for (n, row) in rows.iter_mut().enumerate() {
        row.line_number = n as i32 + 1;
    }
    rows
}

impl Sst {
    /// The figures of a return for `period` under `registration`, read at
    /// `pins`.
    pub(crate) async fn compute(
        &self,
        s: &Scope,
        company: i64,
        period: &Period,
        registration: &Registration,
        pins: Pins,
    ) -> Result<Figures> {
        let tax_type = period.tax_type;
        let (from, to) = (period.period_start, period.period_end);
        let payment_basis =
            tax_type == TaxType::ServiceTax && registration.service_tax_basis == "payment";
        let mapping_number = match pins.through_mapping_number {
            Some(number) => number,
            None => {
                let c = self.db.get().await.map_err(Error::from)?;
                mappings::last_number(&**c, s.tenant(), company).await?
            }
        };
        let book = self.ledger.get_book(s, company).await?;
        let totals = self
            .ledger
            .account_totals(
                s,
                company,
                &TotalsQuery {
                    from: Some(from),
                    to: Some(to),
                    through_entry_number: pins.through_entry_number,
                    exclude_closing: false,
                },
            )
            .await?;
        let through = totals.through_entry_number;
        let receivables = self
            .receivables
            .tax_totals(
                s,
                company,
                &TaxOptions {
                    from: Some(from),
                    to: Some(to),
                    through_entry_number: Some(through),
                },
            )
            .await?;
        let payables = self
            .payables
            .tax_totals(
                s,
                company,
                &TaxQuery {
                    from: Some(from),
                    to: Some(to),
                    through_entry_number: Some(through),
                },
            )
            .await?;
        let received = if payment_basis {
            self.receivables
                .received_tax_totals(
                    s,
                    company,
                    &ReceivedTaxOptions {
                        from,
                        to,
                        unpaid_months: UNPAID_MONTHS,
                        through_entry_number: Some(through),
                    },
                )
                .await?
        } else {
            Vec::new()
        };
        let chart = self.ledger.list_accounts(s, company).await?;
        let mappings = {
            let c = self.db.get().await.map_err(Error::from)?;
            mappings::at(&**c, s.tenant(), company, mapping_number).await?
        };

        let accrual: Vec<Row> = receivables
            .into_iter()
            .map(|r| Row {
                sign: if r.kind == "invoice" { 1 } else { -1 },
                source: TaxSource::Receivables,
                document_kind: r.kind,
                measure: "documents".into(),
                prior_period: r.credits_prior_period,
                tax_code_id: r.tax_code_id,
                tax_code: r.tax_code,
                rate: r.tax_rate,
                account_id: r.account_id,
                document_count: r.document_count,
                carried_taxable: r.taxable_amount,
                carried_tax: r.tax_amount,
            })
            .chain(payables.into_iter().map(|r| Row {
                sign: if r.document_type == "bill" { 1 } else { -1 },
                source: TaxSource::Payables,
                document_kind: r.document_type,
                measure: "documents".into(),
                prior_period: false,
                tax_code_id: r.tax_code_id,
                tax_code: r.tax_code,
                rate: r.rate,
                account_id: r.account_id,
                document_count: r.document_count,
                carried_taxable: r.taxable_amount.amount(),
                carried_tax: r.tax_amount.amount(),
            }))
            .collect();
        let received: Vec<Row> = received
            .into_iter()
            .map(|r| {
                let credited = r.measure == "credited_after_due";
                Row {
                    source: TaxSource::Receivables,
                    document_kind: if credited { "credit_note" } else { "invoice" }.into(),
                    measure: r.measure,
                    prior_period: credited,
                    tax_code_id: r.tax_code_id,
                    tax_code: r.tax_code,
                    rate: r.tax_rate,
                    account_id: r.account_id,
                    document_count: r.document_count,
                    carried_taxable: r.taxable_amount,
                    carried_tax: r.tax_amount,
                    sign: if credited { -1 } else { 1 },
                }
            })
            .collect();

        // On the payment basis, receivables' codes mapped to service tax
        // boxes count what was received instead of what was invoiced.
        let read: Vec<&Row> = if payment_basis {
            accrual
                .iter()
                .filter(|r| !(r.source == TaxSource::Receivables && service(r, &mappings)))
                .chain(received.iter().filter(|r| service(r, &mappings)))
                .collect()
        } else {
            accrual.iter().collect()
        };
        let mut sources = Vec::new();
        let mut unmapped: Vec<UnmappedRow> = Vec::new();
        for row in read {
            match route(row, &mappings, tax_type) {
                Routed::Counted(mut counted) => {
                    counted.line_number = sources.len() as i32 + 1;
                    sources.push(counted);
                }
                Routed::Unmapped(row) => {
                    if !unmapped
                        .iter()
                        .any(|u| u.source == row.source && u.tax_code_id == row.tax_code_id)
                    {
                        unmapped.push(row);
                    }
                }
                Routed::Elsewhere => {}
            }
        }
        let minor_units = book.minor_units.get();
        let boxes = form::figures(
            tax_type,
            &sums(&sources),
            u32::from(minor_units),
        );
        let tax_payable = boxes
            .iter()
            .find(|b| b.key == form::PAYABLE_BOX)
            .and_then(|b| b.tax_amount)
            .unwrap_or_default();
        let ledger: HashMap<i64, (Decimal, Decimal)> = totals
            .rows
            .iter()
            .map(|r| (r.account.id, (r.debits.amount(), r.credits.amount())))
            .collect();
        let accounts: HashMap<i64, (String, String)> = chart
            .into_iter()
            .map(|a| (a.id, (a.code, a.name)))
            .collect();
        Ok(Figures {
            tax_type,
            period_start: from,
            period_end: to,
            service_tax_basis: (tax_type == TaxType::ServiceTax)
                .then(|| registration.service_tax_basis.clone()),
            currency_code: book.currency_code,
            minor_units,
            through_entry_number: through,
            through_mapping_number: mapping_number,
            rate_mismatches: sources.iter().filter(|r| r.rate_mismatch).cloned().collect(),
            accounts: reconcile(&accrual, tax_type, &mappings, &ledger, &accounts),
            boxes,
            tax_payable,
            sources,
            unmapped,
        })
    }
}
