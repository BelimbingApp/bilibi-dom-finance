//! Group totals: an account group takes no postings and stands for what is
//! under it, so its figure is the sum of the accounts below it, added up from
//! the per-account sums PostgreSQL produced for one pinned trial balance.
//! Accounts report their own balance; a group never does.
use crate::{Account, AccountType, ChartBalance, Money, TrialBalanceRow};
use std::collections::HashMap;

/// Every row of `chart` by code with its figure: an account's own balance, a
/// group's total over the accounts beneath it, both on the type's natural side.
pub(crate) fn roll_up(mut chart: Vec<Account>, trial: &[TrialBalanceRow]) -> Vec<ChartBalance> {
    chart.sort_by(|a, b| a.code.cmp(&b.code));
    let own: HashMap<i64, Money> = trial
        .iter()
        .map(|row| {
            (
                row.account.id,
                natural(&row.account, row.debits, row.credits),
            )
        })
        .collect();
    let parents: HashMap<i64, Option<i64>> = chart.iter().map(|a| (a.id, a.parent_id)).collect();
    let mut totals: HashMap<i64, Money> = HashMap::new();
    for account in chart.iter().filter(|a| !a.is_group) {
        let Some(&amount) = own.get(&account.id) else {
            continue;
        };
        // The chart refuses a cycle; the walk is bounded by its size anyway.
        let mut above = account.parent_id;
        for _ in 0..parents.len() {
            let Some(id) = above else { break };
            let total = totals.entry(id).or_insert(Money::ZERO);
            *total = *total + amount;
            above = parents.get(&id).copied().flatten();
        }
    }
    chart
        .into_iter()
        .map(|account| {
            let figure = if account.is_group {
                (
                    None,
                    Some(totals.get(&account.id).copied().unwrap_or(Money::ZERO)),
                )
            } else {
                (
                    Some(own.get(&account.id).copied().unwrap_or(Money::ZERO)),
                    None,
                )
            };
            ChartBalance {
                account,
                balance: figure.0,
                total: figure.1,
            }
        })
        .collect()
}

fn natural(account: &Account, debits: Money, credits: Money) -> Money {
    match account.account_type {
        AccountType::Asset | AccountType::Expense => debits - credits,
        _ => credits - debits,
    }
}
