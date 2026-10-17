//! The `finance_reconciliation_statements` and `_statement_lines` rows read
//! back (Bilimbi `Reconciliation.Statement` and `StatementLine`). A statement
//! and its lines are written by `import.rs` and never changed.
use crate::{ImportMapping, Statement, StatementLine};
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::{MinorUnits, Money};
use rust_decimal::Decimal;
use serde_json::Value;
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "id,bank_account_id,statement_number,file_name,file_sha256,import_mapping,line_count,\
     to_char(start_date,'YYYY-MM-DD'),to_char(end_date,'YYYY-MM-DD'),opening_balance,closing_balance,\
     imported_by_user_id,imported_by_impersonator_id,\
     to_char(imported_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";
const LINE_COLUMNS: &str =
    "id,statement_id,bank_account_id,line_number,to_char(line_date,'YYYY-MM-DD'),\
     description,reference,amount,running_balance";

/// One statement of the company, with its lines in line order when asked.
pub(crate) async fn find(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    statement_id: i64,
    with_lines: bool,
) -> Result<Option<Statement>> {
    let Some(row) = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM finance_reconciliation_statements WHERE tenant_id=$1 AND company_id=$2 AND id=$3"),
            &[&tenant, &company_id, &statement_id],
        )
        .await?
    else {
        return Ok(None);
    };
    let mut statement = from_row(&row)?;
    if with_lines {
        statement.lines = client
            .query(
                &format!("SELECT {LINE_COLUMNS} FROM finance_reconciliation_statement_lines WHERE tenant_id=$1 AND company_id=$2 AND statement_id=$3 ORDER BY line_number"),
                &[&tenant, &company_id, &statement_id],
            )
            .await?
            .iter()
            .map(line_from_row)
            .collect::<Result<_>>()?;
    }
    Ok(Some(statement))
}

/// The bank account's statements after the id `after`, oldest first, without
/// their lines.
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    bank_account_id: i64,
    limit: i64,
    after: i64,
) -> Result<Vec<Statement>> {
    client
        .query(
            &format!("SELECT {COLUMNS} FROM finance_reconciliation_statements WHERE tenant_id=$1 AND company_id=$2 AND bank_account_id=$3 AND id>$4 ORDER BY id LIMIT $5"),
            &[&tenant, &company_id, &bank_account_id, &after, &limit],
        )
        .await?
        .iter()
        .map(from_row)
        .collect()
}

/// An amount read back from a `numeric(20,4)` column: it was accepted as
/// money of the book when it was written, so it fits four places.
pub(crate) fn stored(amount: Decimal) -> Result<Money> {
    Money::new(
        amount,
        MinorUnits::new(4).expect("four is a number of places"),
    )
    .map_err(|_| Error::Internal)
}

fn from_row(row: &Row) -> Result<Statement> {
    let date = |index: usize| {
        row.get::<_, String>(index)
            .parse()
            .map_err(|_| Error::Internal)
    };
    Ok(Statement {
        id: row.get(0),
        bank_account_id: row.get(1),
        statement_number: row.get(2),
        file_name: row.get(3),
        file_sha256: row.get(4),
        import_mapping: serde_json::from_value::<ImportMapping>(row.get::<_, Value>(5))
            .map_err(|_| Error::Internal)?,
        line_count: row.get(6),
        start_date: date(7)?,
        end_date: date(8)?,
        opening_balance: row.get::<_, Option<Decimal>>(9).map(stored).transpose()?,
        closing_balance: row.get::<_, Option<Decimal>>(10).map(stored).transpose()?,
        imported_by_user_id: row.get(11),
        imported_by_impersonator_id: row.get(12),
        imported_at: row
            .get::<_, String>(13)
            .parse()
            .map_err(|_| Error::Internal)?,
        lines: Vec::new(),
    })
}

fn line_from_row(row: &Row) -> Result<StatementLine> {
    Ok(StatementLine {
        id: row.get(0),
        statement_id: row.get(1),
        bank_account_id: row.get(2),
        line_number: row.get(3),
        line_date: row
            .get::<_, String>(4)
            .parse()
            .map_err(|_| Error::Internal)?,
        description: row.get(5),
        reference: row.get(6),
        amount: stored(row.get(7))?,
        running_balance: row.get::<_, Option<Decimal>>(8).map(stored).transpose()?,
    })
}
