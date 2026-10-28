//! The book's currency against Core's currency list (`core/currency`), which
//! owns the ISO 4217 codes, names and decimal places. The ledger keeps only
//! the code and the decimal places the person confirmed.
//!
//! When the list has not been imported (nothing is enabled and the code is
//! not in it), a well-formed code is accepted *unchecked* and the decimal
//! places must be given: the book still opens, and the review says so.
use crate::FieldError;
use bilibi_currency::{Currencies, Currency, Unusable};
use bilibi_database::{Db, Error, Result};
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};

struct Bound {
    db: Db,
    currencies: Arc<Currencies>,
}
static BOUND: OnceLock<Bound> = OnceLock::new();

pub(crate) fn bind(db: Db, currencies: Arc<Currencies>) -> bilibi_module_registry::Result<()> {
    BOUND.set(Bound { db, currencies }).map_err(|_| {
        bilibi_module_registry::RegistryError::Service("duplicate ledger currency binding".into())
    })
}

fn bound() -> Result<&'static Bound> {
    BOUND.get().ok_or(Error::Internal)
}

/// What a code resolved to: a currency of the list, or `None` when the list
/// is unavailable and the code is taken as typed.
pub(crate) type Chosen = Option<Currency>;

fn refusal(why: Unusable) -> FieldError {
    FieldError::new("currency_code", why.message())
}

/// The currency for a code on the write's own transaction. The enabled row is
/// locked until commit, so a concurrent disable waits for the book.
pub(crate) async fn choose_on(
    tx: &bilibi_database::MutationTransaction<'_>,
    scope: &Scope,
    code: &str,
) -> Result<std::result::Result<Chosen, FieldError>> {
    let currencies = &bound()?.currencies;
    Ok(match currencies.check(tx, scope, code).await? {
        Ok(currency) => Ok(Some(currency)),
        Err(Unusable::Unknown) if currencies.enabled(tx.sql(), scope).await?.is_empty() => Ok(None),
        Err(why) => Err(refusal(why)),
    })
}

/// The same answer for a preview, which only reads. `TODO(platform)`: Core's
/// facade takes a `GenericClient`, which the handed `OperationClient` is not,
/// so a preview reads on a pooled connection; a commit uses `choose_on`.
pub(crate) async fn choose(
    scope: &Scope,
    code: &str,
) -> Result<std::result::Result<Chosen, FieldError>> {
    let bound = bound()?;
    let client = bound.db.get().await.map_err(Error::from)?;
    let currencies = &bound.currencies;
    Ok(match currencies.get(&**client, scope, code).await? {
        Some(currency) if currency.withdrawn => Err(refusal(Unusable::Withdrawn)),
        Some(currency) if !currency.enabled => Err(refusal(Unusable::NotEnabled)),
        Some(currency) => Ok(Some(currency)),
        None if currencies.enabled(&**client, scope).await?.is_empty() => Ok(None),
        None => Err(refusal(Unusable::Unknown)),
    })
}

/// The decimal places a book opens with when none are given, or the field
/// error when the list cannot say.
pub(crate) fn default_units(chosen: &Chosen) -> std::result::Result<i64, FieldError> {
    chosen
        .as_ref()
        .map(|currency| i64::from(currency.minor_units))
        .ok_or_else(|| {
            FieldError::new(
                "minor_units",
                "is required: the currency list is not available to suggest it",
            )
        })
}

/// What a review shows about the choice: the currency's name, its standard
/// decimal places, and whether the chosen places differ from them.
pub(crate) fn facts(code: &str, units: i64, chosen: &Chosen) -> Value {
    match chosen {
        Some(currency) => json!({
            "currency_code": code,
            "currency_name": currency.name,
            "minor_units": units,
            "standard_minor_units": currency.minor_units,
            "nonstandard_minor_units": units != i64::from(currency.minor_units),
            "reference": "iso4217",
        }),
        None => json!({
            "currency_code": code,
            "currency_name": null,
            "minor_units": units,
            "standard_minor_units": null,
            "nonstandard_minor_units": null,
            "reference": "unchecked",
        }),
    }
}

/// What the Book page and the discovery read offer: the enabled currencies
/// that match `search` (code or name), and the company's base currency, else
/// the tenant's, as the proposal. `TODO(platform)`: pooled for the same reason
/// as `choose`; a read holds no company lock.
pub(crate) async fn options(scope: &Scope, company: i64, search: &str) -> Result<Value> {
    let bound = bound()?;
    let client = bound.db.get().await.map_err(Error::from)?;
    let client = &**client;
    let currencies = &bound.currencies;
    let all = currencies.enabled(client, scope).await?;
    let company = OrganisationId::new(company).map_err(|_| Error::Invalid)?;
    let proposed = match currencies.base(client, scope, Some(company)).await? {
        Some(base) if base.enabled && !base.withdrawn => {
            let own = currencies.own_base(client, scope, company).await?.is_some();
            Some(json!({
                "code": base.code,
                "name": base.name,
                "minor_units": base.minor_units,
                "source": if own { "company" } else { "tenant" },
            }))
        }
        _ => None,
    };
    let needle = search.trim().to_lowercase();
    let listed: Vec<Value> = all
        .iter()
        .filter(|c| {
            needle.is_empty()
                || c.code.to_lowercase().contains(&needle)
                || c.name.to_lowercase().contains(&needle)
        })
        .take(300)
        .map(|c| json!({"code": c.code, "name": c.name, "minor_units": c.minor_units}))
        .collect();
    Ok(json!({
        "reference": if all.is_empty() { "unavailable" } else { "iso4217" },
        "proposed": proposed,
        "currencies": listed,
    }))
}
