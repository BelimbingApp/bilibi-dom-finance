//! Bilimbi.Finance.FixedAssets register_report/depreciation_schedule_report.
//! Four independent immutable-stream pins reproduce backdated activity too.
use crate::*;
use activity::Activity;
use bilibi_finance_ledger::{AccountTotal, MinorUnits, TotalsQuery};
use std::collections::{BTreeMap, BTreeSet};
use store::{int, Table};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportPins {
    pub through_entry_number: Option<i64>,
    pub through_asset_number: Option<i64>,
    pub through_event_number: Option<i64>,
    pub through_category_revision_id: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationKind {
    Balance,
    Activity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reconciliation {
    pub account_id: i64,
    pub kind: ReconciliationKind,
    pub ledger_balance: Money,
    pub register_balance: Money,
    pub difference: Money,
}
#[derive(Clone, Debug)]
pub struct RegisterRow {
    pub asset: Asset,
    pub revision: CategoryRevision,
    pub accumulated: Money,
    pub scheduled_depreciation: Money,
    pub carrying_value: Money,
    pub disposed: bool,
    pub voided: bool,
    pub disposal_gain: Money,
    pub schedule: Vec<ScheduleRow>,
}
#[derive(Clone, Debug)]
pub struct RegisterReport {
    pub company_id: i64,
    pub as_of: Date,
    pub currency_code: String,
    pub minor_units: MinorUnits,
    pub through_entry_number: i64,
    pub through_asset_number: i64,
    pub through_event_number: i64,
    pub through_category_revision_id: i64,
    pub rows: Vec<RegisterRow>,
    pub reconciliation: Vec<Reconciliation>,
}
impl RegisterReport {
    pub fn pins(&self) -> ReportPins {
        ReportPins {
            through_entry_number: Some(self.through_entry_number),
            through_asset_number: Some(self.through_asset_number),
            through_event_number: Some(self.through_event_number),
            through_category_revision_id: Some(self.through_category_revision_id),
        }
    }
}
impl FixedAssets {
    /// Reproduce with the same date and all four returned boundaries.
    pub async fn register_report(
        &self,
        s: &Scope,
        company: i64,
        date: Date,
        pins: &ReportPins,
    ) -> Result<RegisterReport> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let tx = c
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let report = self.report_on(&*tx, s, company, date, pins).await?;
        tx.commit().await?;
        Ok(report)
    }
    pub async fn depreciation_schedule_report(
        &self,
        s: &Scope,
        company: i64,
        date: Date,
        pins: &ReportPins,
    ) -> Result<RegisterReport> {
        self.register_report(s, company, date, pins).await
    }
    pub(crate) async fn report_on(
        &self,
        c: &(impl GenericClient + Sync),
        s: &Scope,
        company: i64,
        date: Date,
        pins: &ReportPins,
    ) -> Result<RegisterReport> {
        authorize(c, s, company, VIEW).await?;
        if [
            pins.through_entry_number,
            pins.through_asset_number,
            pins.through_event_number,
            pins.through_category_revision_id,
        ]
        .into_iter()
        .flatten()
        .any(|n| n < 0)
        {
            return Err(FixedAssetsError::InvalidOption);
        }
        let query = TotalsQuery {
            to: Some(date),
            through_entry_number: pins.through_entry_number,
            ..Default::default()
        };
        let totals = self.ledger.account_totals_on(c, s, company, &query).await?;
        let through = totals.through_entry_number;
        let flows = self
            .ledger
            .account_totals_on(
                c,
                s,
                company,
                &TotalsQuery {
                    through_entry_number: Some(through),
                    exclude_closing: true,
                    ..query
                },
            )
            .await?;
        let assets = store::all(c, Table::Asset, s.tenant(), company).await?;
        let revisions = store::all(c, Table::Revision, s.tenant(), company).await?;
        let mut activity = Activity::load(c, s, company, totals.minor_units).await?;
        let asset_pin = pins.through_asset_number.unwrap_or(
            assets
                .iter()
                .filter_map(|a| a["asset_number"].as_i64())
                .max()
                .unwrap_or(0),
        );
        let event_pin = pins.through_event_number.unwrap_or(
            activity
                .events
                .iter()
                .map(|e| e.event_number)
                .max()
                .unwrap_or(0),
        );
        let revision_pin = pins.through_category_revision_id.unwrap_or(
            revisions
                .iter()
                .filter_map(|r| r["id"].as_i64())
                .max()
                .unwrap_or(0),
        );
        activity.events.retain(|e| {
            e.event_number <= event_pin
                && e.effective_date <= date
                && e.entry_number.is_none_or(|n| n <= through)
        });
        activity.runs.retain(|r| {
            r.period.last_of_month() <= date && r.entry_number.is_none_or(|n| n <= through)
        });
        // Lifecycle state comes from the bounded events, not today's ledger reversal link.
        for run in &mut activity.runs {
            run.reversed = activity
                .events
                .iter()
                .any(|e| e.run_id == Some(run.id) && e.action == EventAction::RunReversed);
        }
        let mut revision_map = BTreeMap::new();
        for v in revisions
            .iter()
            .filter(|r| r["id"].as_i64().is_some_and(|n| n <= revision_pin))
        {
            let id = int(v, "id")?;
            revision_map.insert(id, store::revision(c, s.tenant(), company, id).await?);
        }
        let mut rows = Vec::new();
        for v in assets {
            let a = store::asset(&v, totals.minor_units)?;
            if a.asset_number > asset_pin || a.entry_number > through || a.acquisition_date > date {
                continue;
            }
            let Some(revision) = revision_map.get(&a.category_revision_id) else {
                continue;
            };
            let voided = activity.voided(a.id);
            let disposal = activity.disposal(a.id);
            let accumulated = if voided {
                Money::ZERO
            } else {
                activity
                    .runs
                    .iter()
                    .filter(|r| !r.reversed)
                    .flat_map(|r| &r.items)
                    .filter(|i| i.asset_id == a.id)
                    .map(|i| i.amount)
                    .sum()
            };
            let schedule = schedule::rows(&a, totals.minor_units)?;
            let scheduled_depreciation = if voided {
                Money::ZERO
            } else {
                schedule
                    .iter()
                    .filter(|r| r.period.last_of_month() <= date)
                    .map(|r| r.amount)
                    .sum()
            };
            let carrying_value = if voided || disposal.is_some() {
                Money::ZERO
            } else {
                a.cost - accumulated
            };
            let disposal_gain = if let Some(e) = disposal {
                e.proceeds.ok_or(Error::Invalid)?
                    - (a.cost - e.accumulated.ok_or(Error::Invalid)?)
            } else {
                Money::ZERO
            };
            rows.push(RegisterRow {
                asset: a,
                revision: revision.clone(),
                accumulated,
                scheduled_depreciation,
                carrying_value,
                disposed: disposal.is_some(),
                voided,
                disposal_gain,
                schedule,
            });
        }
        rows.sort_by_key(|r| r.asset.asset_number);
        let reconciliation = reconcile(&rows, &totals.rows, &flows.rows, revision_map.values());
        Ok(RegisterReport {
            company_id: company,
            as_of: date,
            currency_code: totals.currency_code,
            minor_units: totals.minor_units,
            through_entry_number: through,
            through_asset_number: asset_pin,
            through_event_number: event_pin,
            through_category_revision_id: revision_pin,
            rows,
            reconciliation,
        })
    }
}
fn reconcile<'a>(
    rows: &[RegisterRow],
    totals: &[AccountTotal],
    flows: &[AccountTotal],
    revisions: impl Iterator<Item = &'a CategoryRevision>,
) -> Vec<Reconciliation> {
    let mut expected = BTreeMap::<i64, Money>::new();
    let mut add = |id, amount| {
        let total = expected.entry(id).or_insert(Money::ZERO);
        *total = *total + amount;
    };
    for row in rows.iter().filter(|r| !r.voided) {
        let r = &row.revision.defaults;
        add(r.expense_account_id, row.accumulated);
        if row.disposed {
            if row.disposal_gain.is_positive() {
                add(r.gain_account_id, Money::ZERO - row.disposal_gain);
            } else {
                add(r.loss_account_id, Money::ZERO - row.disposal_gain);
            }
        } else {
            add(r.cost_account_id, row.asset.cost);
            add(r.accumulated_account_id, Money::ZERO - row.accumulated);
        }
    }
    let mut stocks = BTreeSet::new();
    let mut activities = BTreeSet::new();
    for r in revisions {
        let r = &r.defaults;
        stocks.extend([r.cost_account_id, r.accumulated_account_id]);
        activities.extend([r.expense_account_id, r.gain_account_id, r.loss_account_id]);
    }
    stocks
        .iter()
        .map(|id| (*id, ReconciliationKind::Balance))
        .chain(
            activities
                .iter()
                .map(|id| (*id, ReconciliationKind::Activity)),
        )
        .map(|(id, kind)| {
            let source = if kind == ReconciliationKind::Balance {
                totals
            } else {
                flows
            };
            let actual = source
                .iter()
                .find(|r| r.account.id == id)
                .map_or(Money::ZERO, |r| r.debits - r.credits);
            let registered = expected.get(&id).copied().unwrap_or(Money::ZERO);
            Reconciliation {
                account_id: id,
                kind,
                ledger_balance: actual,
                register_balance: registered,
                difference: actual - registered,
            }
        })
        .collect()
}
