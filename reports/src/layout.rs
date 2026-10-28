//! A company's report layout (Bilimbi `Bilimbi.Finance.Reports.Layout`): the
//! `finance.reports.sections` setting, an ordered JSON array of sections, each
//! naming the report it belongs to, its heading and the codes of the accounts
//! it gathers:
//!
//! ```json
//! [{"report": "balance_sheet", "name": "...", "accounts": ["...", "..."]}]
//! ```
//!
//! Headings and account codes are the company's own data. A listed account
//! group brings everything under it in the chart with it, unless a descendant
//! is listed itself; the nearest listed row decides, and within one report the
//! first section that lists a code keeps it. A group is never a report row: it
//! takes no postings, so its figure is the sum of the accounts it gathers.
use bilibi_finance_ledger::Account;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The reports a section may belong to.
pub(crate) const REPORTS: [&str; 3] = ["profit_and_loss", "balance_sheet", "cash_position"];
const NAME_LIMIT: usize = 100;
const CODE_LIMIT: usize = 50;

/// Whether `value` is a valid layout: Base Settings' validator.
pub(crate) fn valid(value: &Value) -> bool {
    value.as_array().is_some_and(|sections| {
        sections.iter().all(|section| {
            let Some(section) = section.as_object() else {
                return false;
            };
            let field = |key: &str| section.get(key);
            section.len() == 3
                && field("report")
                    .and_then(Value::as_str)
                    .is_some_and(|report| REPORTS.contains(&report))
                && field("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| present(name, NAME_LIMIT))
                && field("accounts")
                    .and_then(Value::as_array)
                    .is_some_and(|codes| {
                        codes
                            .iter()
                            .all(|code| code.as_str().is_some_and(|code| present(code, CODE_LIMIT)))
                    })
        })
    })
}

fn present(text: &str, limit: usize) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= limit
}

/// One section of one report: its heading and the account codes it lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Section {
    pub name: String,
    pub codes: Vec<String>,
}

/// The sections of `report`, in order. A stored value that is not a valid
/// layout (one written before validation, or adopted) yields no sections
/// rather than a wrong grouping.
pub(crate) fn sections(value: &Value, report: &str) -> Vec<Section> {
    if !valid(value) {
        return Vec::new();
    }
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|section| section["report"] == report)
        .map(|section| Section {
            name: section["name"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            codes: section["accounts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|code| code.trim().to_owned())
                .collect(),
        })
        .collect()
}

/// Each account's section index in `sections` (none when no listed account
/// covers it), and the listed codes the chart does not have.
pub(crate) fn assign(
    accounts: &[Account],
    sections: &[Section],
) -> (BTreeMap<i64, usize>, Vec<String>) {
    let by_code: HashMap<&str, &Account> = accounts.iter().map(|a| (a.code.as_str(), a)).collect();
    let by_id: HashMap<i64, &Account> = accounts.iter().map(|a| (a.id, a)).collect();
    let mut listed: HashMap<&str, usize> = HashMap::new();
    for (index, section) in sections.iter().enumerate() {
        for code in &section.codes {
            listed.entry(code.as_str()).or_insert(index);
        }
    }
    let mut unmatched: Vec<String> = Vec::new();
    for code in sections.iter().flat_map(|section| &section.codes) {
        if !by_code.contains_key(code.as_str()) && !unmatched.contains(code) {
            unmatched.push(code.clone());
        }
    }
    let membership = accounts
        .iter()
        .filter_map(|account| nearest(account, &listed, &by_id).map(|index| (account.id, index)))
        .collect();
    (membership, unmatched)
}

// The chart refuses a cycle, but the walk is bounded by the chart's size so a
// malformed parent chain can never loop.
fn nearest(
    account: &Account,
    listed: &HashMap<&str, usize>,
    by_id: &HashMap<i64, &Account>,
) -> Option<usize> {
    let mut current = Some(account);
    for _ in 0..=by_id.len() {
        let account = current?;
        if let Some(index) = listed.get(account.code.as_str()) {
            return Some(*index);
        }
        current = account.parent_id.and_then(|id| by_id.get(&id).copied());
    }
    None
}
