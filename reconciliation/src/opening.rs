//! The account page's list/open operations (Bilimbi `Reconciliations.open`
//! and `list`). Matching, figures and finalisation remain the NC2 slice.
use crate::{
    authorize, bank_accounts, people, FieldError, Reconciliation, ReconciliationError as E, Result,
    MANAGE_RECONCILIATIONS, VIEW,
};
use bilibi_database::{Change, Error};
use bilibi_finance_ledger::{MinorUnits, Money};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::{GenericClient, Row};

/// A reconciliation's immutable opening and its latest event's status.
#[derive(Clone, Debug, Serialize)]
pub struct ReconciliationRecord {
    pub id: i64,
    pub bank_account_id: i64,
    pub reconciliation_number: i64,
    #[serde(serialize_with = "crate::as_text")]
    pub end_date: Date,
    pub statement_balance: Money,
    pub status: String,
}
/// The statement date and signed balance to reconcile.
pub struct NewReconciliation {
    pub end_date: Option<Date>,
    pub statement_balance: String,
}
const SELECT: &str = "SELECT r.id,r.bank_account_id,r.reconciliation_number,r.end_date::text,r.statement_balance,\
    CASE (SELECT e.action FROM finance_reconciliation_reconciliation_events e \
        WHERE e.tenant_id=r.tenant_id AND e.company_id=r.company_id AND e.reconciliation_id=r.id ORDER BY e.event_number DESC LIMIT 1) \
        WHEN 'finalised' THEN 'finalised' WHEN 'abandoned' THEN 'abandoned' ELSE 'open' END \
    FROM finance_reconciliation_reconciliations r WHERE r.tenant_id=$1 AND r.company_id=$2";
fn record(row: &Row) -> bilibi_database::Result<ReconciliationRecord> {
    Ok(ReconciliationRecord {
        id: row.get(0),
        bank_account_id: row.get(1),
        reconciliation_number: row.get(2),
        end_date: row
            .get::<_, String>(3)
            .parse()
            .map_err(|_| Error::Internal)?,
        statement_balance: Money::new(row.get(4), MinorUnits::new(4).ok_or(Error::Internal)?)
            .map_err(|_| Error::Internal)?,
        status: row.get(5),
    })
}
async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    account: i64,
) -> bilibi_database::Result<Vec<ReconciliationRecord>> {
    client
        .query(
            &format!(
                "{SELECT} AND r.bank_account_id=$3 ORDER BY r.end_date,r.reconciliation_number"
            ),
            &[&tenant, &company, &account],
        )
        .await?
        .iter()
        .map(record)
        .collect()
}
impl Reconciliation {
    /// The bank account's reconciliations, oldest date first; status comes
    /// from its latest event (`finance.reconciliation.view`).
    pub async fn list_reconciliations(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
    ) -> Result<Vec<ReconciliationRecord>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        bank_accounts::find(&**client, scope.tenant(), company_id, bank_account_id)
            .await?
            .ok_or(E::BankAccountNotFound)?;
        Ok(list(&**client, scope.tenant(), company_id, bank_account_id).await?)
    }
    /// Opens one reconciliation, refusing an existing open one or an end
    /// date not after the latest finalised date. The record and opened
    /// event share the ledger's audited company-locked transaction.
    pub async fn open_reconciliation(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
        new: &NewReconciliation,
    ) -> Result<ReconciliationRecord> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .ledger
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_RECONCILIATIONS],
                "finance.reconciliation.open_reconciliation",
            )
            .await?;
        let tenant = scope.tenant();
        let account = bank_accounts::find(tx.sql(), tenant, company_id, bank_account_id)
            .await?
            .ok_or(E::BankAccountNotFound)?;
        if !account.active {
            return Err(E::BankAccountInactive);
        }
        let book = self.ledger.get_book_on(tx.sql(), scope, company_id).await?;
        let mut errors = vec![];
        if new.end_date.is_none() {
            errors.push(FieldError::new("end_date", "can't be blank"));
        }
        let balance = if new.statement_balance.trim().is_empty() {
            errors.push(FieldError::new("statement_balance", "can't be blank"));
            None
        } else {
            match Money::parse(new.statement_balance.trim(), book.minor_units) {
                Ok(v) => Some(v),
                Err(_) => {
                    errors.push(FieldError::new(
                        "statement_balance",
                        "has more decimal places than the book's currency or is invalid",
                    ));
                    None
                }
            }
        };
        let previous = list(tx.sql(), tenant, company_id, bank_account_id).await?;
        if previous.iter().any(|r| r.status == "open") {
            errors.push(FieldError::new(
                "bank_account_id",
                "already has an open reconciliation",
            ));
        }
        if let (Some(date), Some(last)) = (
            new.end_date,
            previous
                .iter()
                .filter(|r| r.status == "finalised")
                .map(|r| r.end_date)
                .max(),
        ) {
            if date <= last {
                errors.push(FieldError::new(
                    "end_date",
                    format!("must be after the last finalised reconciliation, {last}"),
                ));
            }
        }
        if !errors.is_empty() {
            return Err(E::Invalid(errors));
        }
        let end_date = new.end_date.ok_or(Error::Internal)?.to_string();
        let balance = balance.ok_or(Error::Internal)?.amount();
        let (actor, impersonator) = people(scope);
        let result=tx.mutate(move|sql|Box::pin(async move{
            let row=sql.query_one("INSERT INTO finance_reconciliation_reconciliations(tenant_id,company_id,bank_account_id,reconciliation_number,end_date,statement_balance,opened_by_user_id,opened_by_impersonator_id,opened_at) VALUES($1,$2,$3,0,$4::text::date,$5,$6,$7,now() AT TIME ZONE 'UTC') RETURNING id,to_jsonb(finance_reconciliation_reconciliations)", &[&tenant,&company_id,&bank_account_id,&end_date,&balance,&actor,&impersonator]).await?;
            let id:i64=row.get(0);let opening:Value=row.get(1);
            let event=sql.query_one("INSERT INTO finance_reconciliation_reconciliation_events(tenant_id,company_id,reconciliation_id,event_number,action,recorded_by_user_id,recorded_by_impersonator_id,recorded_at) VALUES($1,$2,$3,0,'opened',$4,$5,now() AT TIME ZONE 'UTC') RETURNING id,to_jsonb(finance_reconciliation_reconciliation_events)",&[&tenant,&company_id,&id,&actor,&impersonator]).await?;
            let record=sql.query_one(&format!("{SELECT} AND r.id=$3"),&[&tenant,&company_id,&id]).await?;
            Ok((record_from_row(&record)?,vec![Change{kind:"finance.reconciliation.reconciliation_record".into(),id,old:Value::Null,new:opening},Change{kind:"finance.reconciliation.reconciliation_event".into(),id:event.get(0),old:Value::Null,new:event.get(1)}]))
        })).await?;
        tx.commit().await?;
        Ok(result)
    }
}
fn record_from_row(row: &Row) -> bilibi_database::Result<ReconciliationRecord> {
    record(row)
}
