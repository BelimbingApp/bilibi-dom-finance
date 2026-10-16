//! The `finance_ledger_accounts` rows (Bilimbi `Ledger.Account` with the
//! facade's parent, designation and deactivation rules). Code and type are
//! the account's identity, since entries are posted against them; the
//! facade authorizes and holds the company lock, and these run inside it.
use crate::{Account, AccountChange, AccountType, FieldError, Money, NewAccount};
use bilibi_database::Result;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const COLUMNS: &str = "id,code,name,account_type,parent_id,active,retained_earnings";
const CODE_LIMIT: usize = 50;
const NAME_LIMIT: usize = 200;

/// The company's accounts, ordered by code.
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Vec<Account>> {
    let rows = client
        .query(
            &format!("SELECT {COLUMNS} FROM finance_ledger_accounts WHERE tenant_id=$1 AND company_id=$2 ORDER BY code"),
            &[&tenant, &company_id],
        )
        .await?;
    Ok(rows.iter().map(from_row).collect())
}

/// One account of the company, or `None` for a missing or other company's id.
pub(crate) async fn find(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    account_id: i64,
) -> Result<Option<Account>> {
    let row = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM finance_ledger_accounts WHERE tenant_id=$1 AND company_id=$2 AND id=$3"),
            &[&tenant, &company_id, &account_id],
        )
        .await?;
    Ok(row.as_ref().map(from_row))
}

/// The account that carries the retained-earnings designation, if any.
async fn designated(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<Option<Account>> {
    let row = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM finance_ledger_accounts WHERE tenant_id=$1 AND company_id=$2 AND retained_earnings"),
            &[&tenant, &company_id],
        )
        .await?;
    Ok(row.as_ref().map(from_row))
}

/// The account's posted debits less its credits. Entries arrive in the next
/// version of this module (`finance_ledger_entry_lines`); until then no
/// account has postings, so every balance is zero.
// TODO(finance/ledger 002 entries): sum finance_ledger_entry_lines for the account.
async fn balance(
    _client: &impl GenericClient,
    _tenant: i64,
    _company_id: i64,
    _account_id: i64,
) -> Result<Money> {
    Ok(Money::ZERO)
}

/// Whether grouping `account_id` under `parent_id` would place it under
/// itself: the parent's chain of parents reaches the account.
async fn cycles(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    parent_id: i64,
    account_id: i64,
) -> Result<bool> {
    let row = client
        .query_opt(
            "WITH RECURSIVE chain(id, parent_id, depth) AS (
               SELECT id, parent_id, 1 FROM finance_ledger_accounts WHERE tenant_id=$1 AND company_id=$2 AND id=$3
               UNION ALL
               SELECT a.id, a.parent_id, chain.depth + 1 FROM finance_ledger_accounts a
               JOIN chain ON a.id = chain.parent_id AND a.company_id=$2
               WHERE chain.depth < 1000
             ) SELECT 1 FROM chain WHERE id=$4 LIMIT 1",
            &[&tenant, &company_id, &parent_id, &account_id],
        )
        .await?;
    Ok(row.is_some())
}

/// The create attributes `Account.create_changeset/2` accepts, trimmed, with
/// the parent checked, or the field errors it reports.
#[derive(Clone, Debug)]
pub(crate) struct ValidAccount {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent_id: Option<i64>,
    pub active: bool,
    pub retained_earnings: bool,
}
pub(crate) async fn validate_new(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    new: &NewAccount,
) -> Result<std::result::Result<ValidAccount, Vec<FieldError>>> {
    let mut errors = Vec::new();
    let code = new.code.trim().to_owned();
    if code.is_empty() {
        errors.push(FieldError::new("code", "can't be blank"));
    } else if code.chars().count() > CODE_LIMIT {
        errors.push(FieldError::new(
            "code",
            format!("should be at most {CODE_LIMIT} character(s)"),
        ));
    } else if find_by_code(client, tenant, company_id, &code)
        .await?
        .is_some()
    {
        errors.push(FieldError::new("code", "has already been taken"));
    }
    let name = match name(&new.name) {
        Ok(name) => name,
        Err(error) => {
            errors.push(error);
            String::new()
        }
    };
    if new.retained_earnings && new.account_type != AccountType::Equity {
        errors.push(FieldError::new(
            "retained_earnings",
            "must be an equity account",
        ));
    }
    if let Some(parent_id) = new.parent_id {
        if let Some(error) = parent_error(
            client,
            tenant,
            company_id,
            parent_id,
            new.account_type,
            None,
        )
        .await?
        {
            errors.push(error);
        }
    }
    Ok(if errors.is_empty() {
        Ok(ValidAccount {
            code,
            name,
            account_type: new.account_type,
            parent_id: new.parent_id,
            active: new.active,
            retained_earnings: new.retained_earnings,
        })
    } else {
        Err(errors)
    })
}

/// The account as it will be after `change` (`Account.update_changeset/2`
/// over the current row, with the facade's parent and deactivation rules),
/// or the field errors.
pub(crate) async fn validate_change(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    current: &Account,
    change: &AccountChange,
    shown_units: crate::MinorUnits,
) -> Result<std::result::Result<Account, Vec<FieldError>>> {
    let mut errors = Vec::new();
    let mut next = current.clone();
    if let Some(value) = &change.name {
        match name(value) {
            Ok(name) => next.name = name,
            Err(error) => errors.push(error),
        }
    }
    if let Some(active) = change.active {
        next.active = active;
    }
    if let Some(retained_earnings) = change.retained_earnings {
        next.retained_earnings = retained_earnings;
    }
    if next.retained_earnings && next.account_type != AccountType::Equity {
        errors.push(FieldError::new(
            "retained_earnings",
            "must be an equity account",
        ));
    }
    if let Some(parent_id) = change.parent_id {
        next.parent_id = parent_id;
        if let Some(parent_id) = parent_id {
            if let Some(error) = parent_error(
                client,
                tenant,
                company_id,
                parent_id,
                current.account_type,
                Some(current.id),
            )
            .await?
            {
                errors.push(error);
            }
        }
    }
    // An account is deactivated only once its balance is zero, so what is
    // still owed on or by it is never left where nothing can be posted.
    if current.active && !next.active {
        let balance = balance(client, tenant, company_id, current.id).await?;
        if !balance.is_zero() {
            errors.push(FieldError::new(
                "active",
                format!(
                    "account {} has a balance of {}; it can be deactivated once its balance is zero",
                    current.code,
                    balance.abs().shown(shown_units)
                ),
            ));
        }
    }
    Ok(if errors.is_empty() {
        Ok(next)
    } else {
        Err(errors)
    })
}

/// Why `parent_id` cannot group an account of `account_type`: it groups
/// accounts of its own type, in this company, without a cycle.
async fn parent_error(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    parent_id: i64,
    account_type: AccountType,
    account_id: Option<i64>,
) -> Result<Option<FieldError>> {
    let Some(parent) = find(client, tenant, company_id, parent_id).await? else {
        return Ok(Some(FieldError::new(
            "parent_id",
            "is not an account of this company",
        )));
    };
    if parent.account_type != account_type {
        return Ok(Some(FieldError::new(
            "parent_id",
            "must have the same account type",
        )));
    }
    if let Some(account_id) = account_id {
        if cycles(client, tenant, company_id, parent_id, account_id).await? {
            return Ok(Some(FieldError::new(
                "parent_id",
                "would place the account under itself",
            )));
        }
    }
    Ok(None)
}

fn name(value: &str) -> std::result::Result<String, FieldError> {
    let name = value.trim();
    if name.is_empty() {
        Err(FieldError::new("name", "can't be blank"))
    } else if name.chars().count() > NAME_LIMIT {
        Err(FieldError::new(
            "name",
            format!("should be at most {NAME_LIMIT} character(s)"),
        ))
    } else {
        Ok(name.to_owned())
    }
}

async fn find_by_code(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    code: &str,
) -> Result<Option<i64>> {
    let row = client
        .query_opt(
            "SELECT id FROM finance_ledger_accounts WHERE tenant_id=$1 AND company_id=$2 AND code=$3",
            &[&tenant, &company_id, &code],
        )
        .await?;
    Ok(row.map(|row| row.get(0)))
}

/// One account carries the retained-earnings designation: designating
/// another releases the current one first, in the same transaction. Returns
/// the released account's `(id, before, after)` for the audit record.
pub(crate) async fn release_designation(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    except: Option<i64>,
) -> Result<Option<(i64, Value, Value)>> {
    let Some(current) = designated(client, tenant, company_id).await? else {
        return Ok(None);
    };
    if except == Some(current.id) {
        return Ok(None);
    }
    let row = client
        .query_one(
            &format!(
                "UPDATE finance_ledger_accounts SET retained_earnings=false,updated_at=now() AT TIME ZONE 'UTC' \
                 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {COLUMNS}"
            ),
            &[&tenant, &company_id, &current.id],
        )
        .await?;
    Ok(Some((
        current.id,
        to_json(&current),
        to_json(&from_row(&row)),
    )))
}

pub(crate) async fn insert(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    account: &ValidAccount,
) -> Result<Account> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO finance_ledger_accounts(tenant_id,company_id,code,name,account_type,parent_id,active,retained_earnings,inserted_at,updated_at) \
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,now() AT TIME ZONE 'UTC',now() AT TIME ZONE 'UTC') RETURNING {COLUMNS}"
            ),
            &[
                &tenant,
                &company_id,
                &account.code,
                &account.name,
                &account.account_type.as_str(),
                &account.parent_id,
                &account.active,
                &account.retained_earnings,
            ],
        )
        .await?;
    Ok(from_row(&row))
}

/// Writes the fields `update_changeset/2` may change; code and type are
/// never sent, and the database refuses them anyway.
pub(crate) async fn update(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    next: &Account,
) -> Result<Account> {
    let row = client
        .query_one(
            &format!(
                "UPDATE finance_ledger_accounts SET name=$4,parent_id=$5,active=$6,retained_earnings=$7,updated_at=now() AT TIME ZONE 'UTC' \
                 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING {COLUMNS}"
            ),
            &[
                &tenant,
                &company_id,
                &next.id,
                &next.name,
                &next.parent_id,
                &next.active,
                &next.retained_earnings,
            ],
        )
        .await?;
    Ok(from_row(&row))
}

pub(crate) fn to_json(account: &Account) -> Value {
    json!({
        "id": account.id,
        "code": account.code,
        "name": account.name,
        "account_type": account.account_type.as_str(),
        "parent_id": account.parent_id,
        "active": account.active,
        "retained_earnings": account.retained_earnings,
    })
}

fn from_row(row: &Row) -> Account {
    let account_type: String = row.get("account_type");
    Account {
        id: row.get("id"),
        code: row.get("code"),
        name: row.get("name"),
        // The column's CHECK constraint admits only the five types.
        account_type: AccountType::parse(&account_type).unwrap_or(AccountType::Asset),
        parent_id: row.get("parent_id"),
        active: row.get("active"),
        retained_earnings: row.get("retained_earnings"),
    }
}
