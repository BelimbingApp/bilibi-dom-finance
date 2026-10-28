//! Starter charts: a template is data in `templates/*.json`, never a code
//! identifier. A person (or an agent) previews one and applies it to a company
//! whose chart is empty; the groups and accounts it creates are then the
//! company's own, with no link back, so editing a chart never reaches another
//! company that started from the same template, and a later template edit never
//! reaches a book. Applying never changes a chart that has accounts.
use crate::{accounts, AccountType, ChartTemplate, ChartTemplateRow, LedgerError};
use bilibi_agent_api::OperationClient;
use bilibi_database::Result;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

const FILES: [&str; 1] = [include_str!("../templates/my-small-business.json")];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    key: String,
    name: String,
    review: String,
    description: String,
    #[serde(default)]
    notes: Vec<String>,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    code: String,
    name: String,
    account_type: String,
    #[serde(default)]
    group: bool,
    parent: Option<String>,
    #[serde(default)]
    retained_earnings: bool,
}

/// Every template, parsed and checked once: parents come first, are groups of
/// the same type, codes are unique, and at most one equity account is
/// designated for retained earnings.
pub(crate) fn all() -> &'static [ChartTemplate] {
    static ALL: OnceLock<Vec<ChartTemplate>> = OnceLock::new();
    ALL.get_or_init(|| {
        FILES
            .iter()
            .map(|source| parse(source).unwrap_or_else(|why| panic!("starter chart: {why}")))
            .collect()
    })
}

pub(crate) fn find(key: &str) -> Option<&'static ChartTemplate> {
    all().iter().find(|template| template.key == key)
}

fn parse(source: &str) -> std::result::Result<ChartTemplate, String> {
    let file: File = serde_json::from_str(source).map_err(|e| e.to_string())?;
    let mut seen: BTreeMap<&str, (AccountType, bool)> = BTreeMap::new();
    let mut retained = 0;
    let mut rows = Vec::with_capacity(file.rows.len());
    for row in &file.rows {
        let account_type = AccountType::parse(&row.account_type)
            .ok_or_else(|| format!("{}: unknown account type", row.code))?;
        if seen.insert(&row.code, (account_type, row.group)).is_some() {
            return Err(format!("{}: code repeated", row.code));
        }
        if let Some(parent) = &row.parent {
            match seen.get(parent.as_str()) {
                Some((parent_type, true)) if *parent_type == account_type => {}
                _ => {
                    return Err(format!(
                        "{}: parent {parent} is not an earlier group of its type",
                        row.code
                    ))
                }
            }
        }
        if row.retained_earnings {
            if row.group || account_type != AccountType::Equity {
                return Err(format!(
                    "{}: retained earnings must be an equity account",
                    row.code
                ));
            }
            retained += 1;
        }
        rows.push(ChartTemplateRow {
            code: row.code.clone(),
            name: row.name.clone(),
            account_type,
            is_group: row.group,
            parent_code: row.parent.clone(),
            retained_earnings: row.retained_earnings,
        });
    }
    if retained > 1 {
        return Err("more than one retained earnings account".into());
    }
    Ok(ChartTemplate {
        key: file.key,
        name: file.name,
        review: file.review,
        description: file.description,
        notes: file.notes,
        rows,
    })
}

/// The template a request names, or the refusal in the words a form shows.
pub(crate) fn require(key: &str) -> std::result::Result<&'static ChartTemplate, LedgerError> {
    find(key).ok_or_else(|| {
        LedgerError::Invalid(vec![crate::FieldError::new(
            "template",
            "is not a starter chart",
        )])
    })
}

/// How many accounts and groups the company has, and which of the template's
/// codes they already use.
pub(crate) async fn existing(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
    template: &ChartTemplate,
) -> Result<(usize, Vec<String>)> {
    let chart = accounts::list(client, tenant, company_id).await?;
    let conflicts = template
        .rows
        .iter()
        .filter(|row| chart.iter().any(|a| a.code == row.code))
        .map(|row| row.code.clone())
        .collect();
    Ok((chart.len(), conflicts))
}
