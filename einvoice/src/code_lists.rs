//! The authority's code lists as reference data, with each company's
//! overrides (Bilimbi `Bilimbi.Finance.Einvoice.CodeLists`).
//!
//! An authority publishes its lists (document types, tax types, item
//! classifications, states, countries, currencies, units, industry codes,
//! payment means). The module ships them in `code_lists/<authority>.json` with
//! the date they were read and each list's source, compiled into the crate: a
//! refreshed list is a new file in a new release, reviewed like code. A
//! company's override, stored in `finance_einvoice_code_overrides`, renames a
//! code, retires it, or adds one the shipped file does not have yet, for that
//! company only, so a list change the authority announces need not wait for a
//! release.
use crate::{CodeOverride, CodeOverrideChange, FieldError};
use bilibi_agent_api::OperationClient;
use bilibi_database::Result;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use tokio_postgres::{GenericClient, Row};

/// One list's active codes: code to description.
pub type Codes = BTreeMap<String, String>;

const SHIPPED_FILES: [&str; 1] = [include_str!("../code_lists/myinvois.json")];

// authority => list => code => description
type Shipped = BTreeMap<String, BTreeMap<String, Codes>>;

fn shipped_all() -> &'static Shipped {
    static SHIPPED: OnceLock<Shipped> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        let mut shipped = Shipped::new();
        for file in SHIPPED_FILES {
            // The files are compiled in and read by the walkthrough fixture,
            // so a malformed one fails the build's proof, never a request.
            let parsed: Value = serde_json::from_str(file).expect("shipped code list is JSON");
            let authority = parsed["authority"].as_str().unwrap_or_default().to_owned();
            let mut lists = BTreeMap::new();
            if let Some(object) = parsed["lists"].as_object() {
                for (list, body) in object {
                    let codes = body["codes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|pair| {
                            Some((pair[0].as_str()?.to_owned(), pair[1].as_str()?.to_owned()))
                        })
                        .collect();
                    lists.insert(list.clone(), codes);
                }
            }
            shipped.insert(authority, lists);
        }
        shipped
    })
}

/// The lists in an authority's shipped file, sorted.
pub(crate) fn lists(authority: &str) -> Vec<&'static str> {
    shipped_all()
        .get(authority)
        .map(|lists| lists.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

/// One shipped list as the authority published it.
pub(crate) fn shipped(authority: &str, list: &str) -> Codes {
    shipped_all()
        .get(authority)
        .and_then(|lists| lists.get(list))
        .cloned()
        .unwrap_or_default()
}

fn from_row(row: &Row) -> CodeOverride {
    CodeOverride {
        list: row.get("list"),
        code: row.get("code"),
        description: row.get("description"),
        active: row.get("active"),
    }
}

async fn overrides(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
    authority: &str,
    list: Option<&str>,
) -> Result<Vec<CodeOverride>> {
    let rows = client
        .query(
            "SELECT list,code,description,active FROM finance_einvoice_code_overrides \
             WHERE tenant_id=$1 AND company_id=$2 AND authority=$3 AND ($4::text IS NULL OR list=$4) \
             ORDER BY list,code",
            &[&tenant, &company_id, &authority, &list],
        )
        .await?;
    Ok(rows.iter().map(from_row).collect())
}

// A retired code leaves the list; any other override names its code (the
// shipped description when the override gives none, else the code itself).
fn apply(overrides: &[CodeOverride], mut codes: Codes) -> Codes {
    for o in overrides {
        if !o.active {
            codes.remove(&o.code);
        } else {
            let fallback = codes
                .get(&o.code)
                .cloned()
                .unwrap_or_else(|| o.code.clone());
            codes.insert(o.code.clone(), o.description.clone().unwrap_or(fallback));
        }
    }
    codes
}

/// The company's active codes in one list: the shipped list with the
/// company's overrides applied.
pub(crate) async fn codes(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
    authority: &str,
    list: &str,
) -> Result<Codes> {
    let overrides = overrides(client, tenant, company_id, authority, Some(list)).await?;
    Ok(apply(&overrides, shipped(authority, list)))
}

/// A code is 1 to 16 letters, digits, dots or dashes.
pub(crate) fn valid_code(code: &str) -> bool {
    (1..=16).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// The override after `change`, or the field errors `CodeOverride.changeset/2`
/// reports, with the row it replaces.
pub(crate) struct ValidOverride {
    pub current: Option<CodeOverride>,
    pub next: CodeOverride,
}
pub(crate) async fn validate(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
    authority: &str,
    list: &str,
    code: &str,
    change: &CodeOverrideChange,
) -> Result<std::result::Result<ValidOverride, Vec<FieldError>>> {
    let mut errors = Vec::new();
    if !valid_code(code) {
        errors.push(FieldError::new(
            "code",
            "must be 1 to 16 letters, digits, dots, or dashes",
        ));
    }
    // Ecto casts a blank string to nil and trims what it keeps.
    let description = change.description.as_ref().map(|description| {
        description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
    });
    if description
        .as_ref()
        .and_then(Option::as_ref)
        .is_some_and(|d| d.chars().count() > 500)
    {
        errors.push(FieldError::new(
            "description",
            "should be at most 500 character(s)",
        ));
    }
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    let current = overrides(client, tenant, company_id, authority, Some(list))
        .await?
        .into_iter()
        .find(|o| o.code == code);
    let next = CodeOverride {
        list: list.to_owned(),
        code: code.to_owned(),
        description: match description {
            Some(description) => description,
            None => current.as_ref().and_then(|o| o.description.clone()),
        },
        active: change
            .active
            .unwrap_or_else(|| current.as_ref().is_none_or(|o| o.active)),
    };
    Ok(Ok(ValidOverride { current, next }))
}

/// Writes the override; the row id. Nothing is written when it changes
/// nothing, so the caller records no `Change` for it.
pub(crate) async fn upsert(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    authority: &str,
    next: &CodeOverride,
) -> Result<i64> {
    let row = client
        .query_one(
            "INSERT INTO finance_einvoice_code_overrides(tenant_id,company_id,authority,list,code,description,active,inserted_at,updated_at) \
             VALUES($1,$2,$3,$4,$5,$6,$7,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC') \
             ON CONFLICT (company_id,authority,list,code) DO UPDATE \
             SET description=excluded.description,active=excluded.active,updated_at=now() AT TIME ZONE 'UTC' \
             RETURNING id",
            &[&tenant, &company_id, &authority, &next.list, &next.code, &next.description, &next.active],
        )
        .await?;
    Ok(row.get(0))
}

pub(crate) fn to_json(o: &CodeOverride) -> Value {
    json!({"list": o.list, "code": o.code, "description": o.description, "active": o.active})
}
