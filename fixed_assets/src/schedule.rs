//! Bilimbi.Finance.FixedAssets.Schedule: full first month; half-up before cap;
//! final month clears the remaining depreciable cost, even for reducing balance.
use crate::*;
use bilibi_finance_ledger::MinorUnits;
use jiff::ToSpan;
use rust_decimal::RoundingStrategy;

pub(crate) fn assumptions(mut a: Assumptions) -> Result<Assumptions> {
    if a.life_months <= 0 {
        return Err(invalid("life_months", "must be greater than 0"));
    }
    if a.life_months > 1200 {
        return Err(invalid("life_months", "must be less than or equal to 1200"));
    }
    if a.method == Method::StraightLine {
        a.annual_rate = None;
    } else {
        let rate = a
            .annual_rate
            .ok_or_else(|| invalid("annual_rate", "can't be blank"))?;
        if rate <= Decimal::ZERO {
            return Err(invalid("annual_rate", "must be greater than 0"));
        }
        if rate > Decimal::from(100) {
            return Err(invalid("annual_rate", "must be less than or equal to 100"));
        }
        if rate.round_dp(4) != rate {
            return Err(invalid("annual_rate", "has more than four decimal places"));
        }
    }
    Ok(a)
}
pub(crate) fn rows(asset: &Asset, units: MinorUnits) -> Result<Vec<ScheduleRow>> {
    let depreciable = (asset.cost - asset.opening_accumulated - asset.residual_value).amount();
    let mut accumulated = Decimal::ZERO;
    let first = asset
        .opening_date
        .unwrap_or(asset.in_service_date)
        .first_of_month();
    (0..asset.assumptions.life_months)
        .map(|month| {
            let remaining = depreciable - accumulated;
            let amount = if month == asset.assumptions.life_months - 1 {
                remaining
            } else if asset.assumptions.method == Method::StraightLine {
                depreciable / Decimal::from(asset.assumptions.life_months)
            } else {
                (asset.cost.amount() - asset.opening_accumulated.amount() - accumulated)
                    * asset.assumptions.annual_rate.ok_or(Error::Invalid)?
                    / Decimal::from(1200)
            };
            let amount = amount
                .round_dp_with_strategy(
                    u32::from(units.get()),
                    RoundingStrategy::MidpointAwayFromZero,
                )
                .min(remaining)
                .max(Decimal::ZERO);
            accumulated += amount;
            let money =
                |v| Money::new(v, units).map_err(|_| FixedAssetsError::Failed(Error::Invalid));
            Ok(ScheduleRow {
                period: first
                    .checked_add(month.months())
                    .map_err(|_| invalid("in_service_date", "is invalid"))?,
                amount: money(amount)?,
                accumulated: money(asset.opening_accumulated.amount() + accumulated)?,
                carrying_value: money(
                    asset.cost.amount() - asset.opening_accumulated.amount() - accumulated,
                )?,
            })
        })
        .collect()
}
