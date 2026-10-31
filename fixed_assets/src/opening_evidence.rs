//! Source-only history and tax schedules. These facts never change book values,
//! prospective schedules or ledger postings; the register's explicit estimates do.
use crate::*;
use bilibi_finance_ledger::MinorUnits;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    #[serde(default)]
    history: Vec<History>,
    #[serde(default)]
    source_period_expense: Vec<Expense>,
    #[serde(default)]
    tax_capital_allowances: Vec<Tax>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct History {
    kind: Kind,
    effective_date: String,
    reference: String,
    note: String,
    amount: Option<String>,
    residual_value: Option<String>,
    remaining_life_months: Option<i32>,
    method: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Impairment,
    ImpairmentReversal,
    Disposal,
    ResidualValueChange,
    EstimateChange,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Expense {
    from: String,
    through: String,
    amount: String,
    reference: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Tax {
    from: String,
    through: String,
    qualifying_expenditure: String,
    allowances: String,
    reference: String,
}
fn fail(message: impl Into<String>) -> FixedAssetsError {
    invalid("source_evidence", message)
}
fn date(value: &str, cutoff: Date) -> Result<Date> {
    let date: Date = value
        .parse()
        .map_err(|_| fail("dates must be YYYY-MM-DD"))?;
    if date.year() < 1 || date >= cutoff {
        return Err(fail(
            "source history and periods must precede the conversion date",
        ));
    }
    Ok(date)
}
fn period(from: &str, through: &str, cutoff: Date) -> Result<()> {
    if date(from, cutoff)? > date(through, cutoff)? {
        return Err(fail("a source period must end on or after its start"));
    }
    Ok(())
}
fn reference(value: &str) -> Result<()> {
    text(value, "source_evidence", 500).map(|_| ())
}
fn amount(value: &str, units: MinorUnits) -> Result<Money> {
    let value = Money::parse(value, units).map_err(|e| fail(e.to_string()))?;
    if value < Money::ZERO {
        return Err(fail("source evidence amounts must be nonnegative"));
    }
    Ok(value)
}
/// Validate the whole export before mutation, including orphan and duplicate IDs.
pub(super) fn read(
    input: &Value,
    cutoff: Date,
    units: MinorUnits,
) -> Result<BTreeMap<String, Value>> {
    let raw = &input["source_evidence"];
    if raw.is_null() {
        return Ok(BTreeMap::new());
    }
    let map = raw
        .as_object()
        .ok_or_else(|| fail("must be an object keyed by source asset ID"))?;
    let max = if input["staged"] == true {
        32 * 1024 * 1024
    } else {
        65536
    };
    if raw.to_string().len() > max || map.len() > 10000 {
        return Err(fail(
            "source evidence is too large; stage smaller source batches",
        ));
    }
    map.iter()
        .map(|(source, value)| {
            let evidence: Evidence = serde_json::from_value(value.clone())
                .map_err(|e| fail(format!("source {source}: {e}")))?;
            if evidence.history.len()
                + evidence.source_period_expense.len()
                + evidence.tax_capital_allowances.len()
                > 200
            {
                return Err(fail(format!(
                    "source {source}: keep at most 200 history and schedule rows"
                )));
            }
            for row in &evidence.history {
                date(&row.effective_date, cutoff)?;
                reference(&row.reference)?;
                text(&row.note, "source_evidence", 2000)?;
                if let Some(v) = &row.amount {
                    amount(v, units)?;
                }
                if let Some(v) = &row.residual_value {
                    amount(v, units)?;
                }
                if row
                    .remaining_life_months
                    .is_some_and(|n| !(0..=1200).contains(&n))
                {
                    return Err(fail("source remaining life must be 0 to 1200 months"));
                }
                if let Some(v) = &row.method {
                    text(v, "source_evidence", 100)?;
                }
                match row.kind {
                    Kind::Impairment | Kind::ImpairmentReversal | Kind::Disposal
                        if row.amount.is_none() =>
                    {
                        return Err(fail("impairment and disposal history require an amount"))
                    }
                    Kind::ResidualValueChange if row.residual_value.is_none() => {
                        return Err(fail(
                            "residual-value history requires the revised residual value",
                        ))
                    }
                    Kind::EstimateChange
                        if row.remaining_life_months.is_none() && row.method.is_none() =>
                    {
                        return Err(fail(
                            "estimate history requires the source method or remaining life",
                        ))
                    }
                    _ => {}
                }
            }
            for row in &evidence.source_period_expense {
                period(&row.from, &row.through, cutoff)?;
                amount(&row.amount, units)?;
                reference(&row.reference)?;
            }
            for row in &evidence.tax_capital_allowances {
                period(&row.from, &row.through, cutoff)?;
                amount(&row.qualifying_expenditure, units)?;
                amount(&row.allowances, units)?;
                reference(&row.reference)?;
            }
            Ok((source.clone(), json!(evidence)))
        })
        .collect()
}
