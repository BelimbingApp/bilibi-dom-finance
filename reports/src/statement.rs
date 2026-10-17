//! Builds a report's figures from the chart and the ledger's per-account
//! totals (Bilimbi `Bilimbi.Finance.Reports.Statement`), one totals map
//! (account id to debits and credits) per column. Nothing here reads data:
//! the facade reads, this arranges and adds.
//!
//! Grouping is by the five account types, the one fixed classification, and
//! inside a type by the company's configured sections (see `layout.rs`); an
//! account no section covers is listed after them under no heading. Each
//! account reports its own postings, so a parent is never counted twice.
use crate::layout::Section as Layout;
use crate::{Group, Row, Section};
use bilibi_finance_ledger::{Account, AccountType, Money};
use std::collections::{BTreeMap, HashMap};

/// One column's totals: each posted account's debits and credits.
pub(crate) type Column = HashMap<i64, (Money, Money)>;

/// An account's balance on its type's natural side: assets and expenses as
/// debits less credits, liabilities, equity and income as credits less debits.
fn natural(account_type: AccountType, (debits, credits): (Money, Money)) -> Money {
    match account_type {
        AccountType::Asset | AccountType::Expense => debits - credits,
        _ => credits - debits,
    }
}

/// Income and expense for each column, and income less expense.
pub(crate) fn profit_and_loss(
    accounts: &[Account],
    sections: &[Layout],
    membership: &BTreeMap<i64, usize>,
    columns: &[Column],
) -> (Vec<Group>, Vec<Money>) {
    let groups = groups(
        &[AccountType::Income, AccountType::Expense],
        accounts,
        sections,
        membership,
        columns,
    );
    let net_income = subtract(&groups[0].totals, &groups[1].totals);
    (groups, net_income)
}

/// Assets, liabilities and equity as of each column's date, and `earnings`:
/// income less expense not yet closed into an equity account. Earnings belong
/// to equity and are part of its totals, so assets equal liabilities and
/// equity.
pub(crate) struct BalanceSheet {
    pub groups: Vec<Group>,
    pub earnings: Vec<Money>,
    pub total_assets: Vec<Money>,
    pub total_liabilities_and_equity: Vec<Money>,
}
pub(crate) fn balance_sheet(
    accounts: &[Account],
    sections: &[Layout],
    membership: &BTreeMap<i64, usize>,
    columns: &[Column],
) -> BalanceSheet {
    let mut groups = groups(
        &[
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
        ],
        accounts,
        sections,
        membership,
        columns,
    );
    let earnings: Vec<Money> = columns
        .iter()
        .map(|totals| {
            accounts
                .iter()
                .filter(|a| matches!(a.account_type, AccountType::Income | AccountType::Expense))
                .filter_map(|a| totals.get(&a.id))
                .map(|(debits, credits)| *credits - *debits)
                .sum()
        })
        .collect();
    groups[2].totals = add(&groups[2].totals, &earnings);
    BalanceSheet {
        total_assets: groups[0].totals.clone(),
        total_liabilities_and_equity: add(&groups[1].totals, &groups[2].totals),
        groups,
        earnings,
    }
}

/// The balances of the accounts the cash sections cover, debits less credits
/// whatever their type, so an overdrawn account counts against the total.
/// Every covered account is listed, with or without postings.
pub(crate) fn cash_position(
    accounts: &[Account],
    sections: &[Layout],
    membership: &BTreeMap<i64, usize>,
    columns: &[Column],
) -> (Vec<Section>, Vec<Money>) {
    let mut covered: Vec<&Account> = accounts
        .iter()
        .filter(|a| membership.contains_key(&a.id))
        .collect();
    covered.sort_by(|a, b| a.code.cmp(&b.code));
    let rows: Vec<(usize, Row)> = covered
        .into_iter()
        .map(|a| (membership[&a.id], row(a, columns, AccountType::Asset)))
        .collect();
    let named = named(sections, rows, columns.len());
    let totals = sum(named.iter().map(|s| s.totals.as_slice()), columns.len());
    (named, totals)
}

fn groups(
    types: &[AccountType],
    accounts: &[Account],
    sections: &[Layout],
    membership: &BTreeMap<i64, usize>,
    columns: &[Column],
) -> Vec<Group> {
    let width = columns.len();
    types
        .iter()
        .map(|&account_type| {
            let mut posted: Vec<&Account> = accounts
                .iter()
                .filter(|a| a.account_type == account_type)
                .filter(|a| columns.iter().any(|totals| totals.contains_key(&a.id)))
                .collect();
            posted.sort_by(|a, b| a.code.cmp(&b.code));
            let (assigned, unassigned): (Vec<_>, Vec<_>) = posted
                .into_iter()
                .map(|a| {
                    (
                        membership.get(&a.id).copied(),
                        row(a, columns, a.account_type),
                    )
                })
                .partition(|(index, _)| index.is_some());
            let mut listed = named(
                sections,
                assigned
                    .into_iter()
                    .map(|(index, row)| (index.unwrap_or_default(), row))
                    .collect(),
                width,
            );
            if !unassigned.is_empty() {
                listed.push(section(
                    None,
                    unassigned.into_iter().map(|(_, row)| row).collect(),
                    width,
                ));
            }
            Group {
                account_type,
                totals: sum(listed.iter().map(|s| s.totals.as_slice()), width),
                sections: listed,
            }
        })
        .collect()
}

/// The configured sections that have rows, in their configured order.
fn named(sections: &[Layout], rows: Vec<(usize, Row)>, width: usize) -> Vec<Section> {
    let mut by_section: BTreeMap<usize, Vec<Row>> = BTreeMap::new();
    for (index, row) in rows {
        by_section.entry(index).or_default().push(row);
    }
    sections
        .iter()
        .enumerate()
        .filter_map(|(index, layout)| {
            by_section
                .remove(&index)
                .map(|rows| section(Some(layout.name.clone()), rows, width))
        })
        .collect()
}

fn row(account: &Account, columns: &[Column], side: AccountType) -> Row {
    Row {
        account_id: account.id,
        code: account.code.clone(),
        name: account.name.clone(),
        account_type: account.account_type,
        parent_id: account.parent_id,
        active: account.active,
        amounts: columns
            .iter()
            .map(|totals| natural(side, totals.get(&account.id).copied().unwrap_or_default()))
            .collect(),
    }
}

fn section(name: Option<String>, rows: Vec<Row>, width: usize) -> Section {
    Section {
        name,
        totals: sum(rows.iter().map(|r| r.amounts.as_slice()), width),
        rows,
    }
}

fn sum<'a>(lists: impl Iterator<Item = &'a [Money]>, width: usize) -> Vec<Money> {
    lists.fold(vec![Money::ZERO; width], |total, list| add(&total, list))
}

pub(crate) fn add(left: &[Money], right: &[Money]) -> Vec<Money> {
    left.iter().zip(right).map(|(l, r)| *l + *r).collect()
}

fn subtract(left: &[Money], right: &[Money]) -> Vec<Money> {
    left.iter().zip(right).map(|(l, r)| *l - *r).collect()
}
