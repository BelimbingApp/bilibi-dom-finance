//! Account totals (Bilimbi `trial_balance/4`, `account_totals/3` and
//! `account_totals_by_owner/3`): PostgreSQL sums the lines per account (and
//! owner), so a report over any number of entries reads one row per account.
//! The lines' `(company_id, account_id)` and `(entry_id, line_number)`
//! indexes serve it. Every read counts the entries numbered up to one pinned
//! entry: the one asked for, or the company's last when none is, read in the
//! same statement as the sums, so a pin names exactly what was counted.
use crate::{
    accounts, periods, Account, AccountTotal, LedgerError, Money, OwnerTotal, TotalsQuery,
};
use jiff::civil::Date;
use rust_decimal::Decimal;
use tokio_postgres::{GenericClient, Row};

/// A validated window: dates both inclusive, the pinned entry number, and
/// whether year-end closing entries and their reversals are left out.
pub(crate) struct Window {
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub through: Option<i64>,
    pub exclude_closing: bool,
}

impl Window {
    /// `InvalidOption` for a negative pin or a `from` after `to`.
    pub(crate) fn new(query: &TotalsQuery) -> Result<Self, LedgerError> {
        if query.through_entry_number.is_some_and(|number| number < 0)
            || matches!((query.from, query.to), (Some(from), Some(to)) if from > to)
        {
            return Err(LedgerError::InvalidOption);
        }
        Ok(Self {
            from: query.from,
            to: query.to,
            through: query.through_entry_number,
            exclude_closing: query.exclude_closing,
        })
    }
}

/// Every account with a posting in the window, by code, with its debit and
/// credit totals; and the entry number counted through.
pub(crate) async fn by_account(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    window: &Window,
) -> Result<(i64, Vec<AccountTotal>), LedgerError> {
    let (through, rows) = sums(client, tenant, company_id, window, false).await?;
    let totals = rows
        .iter()
        .map(|(row, account)| AccountTotal {
            account: account.clone(),
            debits: amount(row, "debits"),
            credits: amount(row, "credits"),
        })
        .collect();
    Ok((through, totals))
}

/// The same sums split by the entries' owner module: one row per account and
/// owner, by code and then owner, a manual entry's (`None`) first.
pub(crate) async fn by_owner(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    window: &Window,
) -> Result<(i64, Vec<OwnerTotal>), LedgerError> {
    let (through, rows) = sums(client, tenant, company_id, window, true).await?;
    let totals = rows
        .iter()
        .map(|(row, account)| OwnerTotal {
            account: account.clone(),
            owner_module: row.get("owner_module"),
            debits: amount(row, "debits"),
            credits: amount(row, "credits"),
        })
        .collect();
    Ok((through, totals))
}

/// One statement: the pin, then the sums joined to their accounts. The pin's
/// row always comes back, with no account when nothing was posted in the
/// window.
async fn sums(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    window: &Window,
    by_owner: bool,
) -> Result<(i64, Vec<(Row, Account)>), LedgerError> {
    let (owner, order) = if by_owner {
        ("e.owner_module", "t.owner_module NULLS FIRST")
    } else {
        ("NULL::varchar", "t.owner_module")
    };
    let rows = client
        .query(
            &format!(
                "WITH pin AS (SELECT COALESCE($3::bigint, (SELECT max(entry_number) \
                   FROM finance_ledger_entries WHERE tenant_id=$1 AND company_id=$2), 0) AS through), \
                 totals AS (SELECT l.account_id, {owner} AS owner_module, \
                   sum(l.debit) AS debits, sum(l.credit) AS credits \
                   FROM finance_ledger_entry_lines l JOIN finance_ledger_entries e ON e.id = l.entry_id \
                   WHERE l.tenant_id=$1 AND l.company_id=$2 AND e.entry_number <= (SELECT through FROM pin) \
                   AND ($4::text IS NULL OR e.entry_date >= $4::text::date) \
                   AND ($5::text IS NULL OR e.entry_date <= $5::text::date) \
                   AND (NOT $6 OR ({not_closing})) \
                   GROUP BY 1, 2) \
                 SELECT pin.through, a.id, a.code, a.name, a.account_type, a.parent_id, a.active, \
                   a.retained_earnings, t.owner_module, t.debits, t.credits \
                 FROM pin LEFT JOIN (totals t JOIN finance_ledger_accounts a ON a.id = t.account_id) ON true \
                 ORDER BY a.code, {order}",
                not_closing = periods::NOT_CLOSING,
            ),
            &[
                &tenant,
                &company_id,
                &window.through,
                &window.from.map(|date| date.to_string()),
                &window.to.map(|date| date.to_string()),
                &window.exclude_closing,
            ],
        )
        .await
        .map_err(bilibi_database::Error::from)?;
    let through = rows.first().map_or(0, |row| row.get("through"));
    let rows = rows
        .into_iter()
        .filter(|row| row.get::<_, Option<i64>>("id").is_some())
        .map(|row| {
            let account = accounts::from_row(&row);
            (row, account)
        })
        .collect();
    Ok((through, rows))
}

fn amount(row: &Row, column: &str) -> Money {
    Money::stored(row.get::<_, Decimal>(column))
}
