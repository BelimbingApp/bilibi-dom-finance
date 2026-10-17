//! The `finance_ledger_books` rows (Bilimbi `Ledger.Book`): one per company,
//! opened once with the currency and its minor units, both fixed for good.
//! The facade authorizes and holds the company lock; these run inside it.
use crate::{Book, FieldError, MinorUnits, NewBook};
use bilibi_agent_api::OperationClient;
use bilibi_database::Result;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "company_id,currency_code,minor_units";

/// The company's book, or `None` before it is opened.
pub(crate) async fn find(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
) -> Result<Option<Book>> {
    let row = client
        .query_opt(
            &format!(
                "SELECT {COLUMNS} FROM finance_ledger_books WHERE tenant_id=$1 AND company_id=$2"
            ),
            &[&tenant, &company_id],
        )
        .await?;
    Ok(row.as_ref().map(from_row))
}

/// The attributes `Book.changeset/2` accepts, trimmed and upcased, or the
/// field errors it reports.
#[derive(Clone, Debug)]
pub(crate) struct ValidBook {
    pub currency_code: String,
    pub minor_units: MinorUnits,
}
pub(crate) fn validate(new: &NewBook) -> std::result::Result<ValidBook, Vec<FieldError>> {
    let mut errors = Vec::new();
    let currency_code = new.currency_code.trim().to_ascii_uppercase();
    if currency_code.is_empty() {
        errors.push(FieldError::new("currency_code", "can't be blank"));
    } else if currency_code.len() != 3 || !currency_code.bytes().all(|c| c.is_ascii_uppercase()) {
        errors.push(FieldError::new("currency_code", "has invalid format"));
    }
    let minor_units = MinorUnits::new(new.minor_units);
    if new.minor_units < 0 {
        errors.push(FieldError::new(
            "minor_units",
            "must be greater than or equal to 0",
        ));
    } else if minor_units.is_none() {
        errors.push(FieldError::new(
            "minor_units",
            format!("must be less than or equal to {}", MinorUnits::MAX),
        ));
    }
    match (errors.is_empty(), minor_units) {
        (true, Some(minor_units)) => Ok(ValidBook {
            currency_code,
            minor_units,
        }),
        _ => Err(errors),
    }
}

/// Inserts the book; the row id and the book as the facade shows it.
pub(crate) async fn insert(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    book: &ValidBook,
) -> Result<(i64, Book)> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO finance_ledger_books(tenant_id,company_id,currency_code,minor_units,inserted_at) \
                 VALUES($1,$2,$3,$4,now() AT TIME ZONE 'UTC') RETURNING id,{COLUMNS}"
            ),
            &[
                &tenant,
                &company_id,
                &book.currency_code,
                &i16::from(book.minor_units.get()),
            ],
        )
        .await?;
    Ok((row.get(0), from_row(&row)))
}

pub(crate) fn to_json(book: &Book) -> Value {
    json!({
        "company_id": book.company_id,
        "currency_code": book.currency_code,
        "minor_units": book.minor_units.get(),
    })
}

// Columns are read by name so the INSERT's leading `id` does not shift them.
fn from_row(row: &Row) -> Book {
    let minor_units: i16 = row.get("minor_units");
    Book {
        company_id: row.get("company_id"),
        currency_code: row.get("currency_code"),
        minor_units: MinorUnits::new(i64::from(minor_units)).unwrap_or(MinorUnits(0)),
    }
}
