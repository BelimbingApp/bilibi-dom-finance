//! Bilimbi.Payables.Balances, TaxReport and TieOut; all figures share a posting fence.
use crate::*;
use bilibi_finance_ledger::{MinorUnits, TotalsQuery};
use std::collections::BTreeMap;

/// A report's posting fence and aging boundaries (0 = not yet due).
#[derive(Clone, Debug)]
pub struct AgingQuery {
    pub through_entry_number: Option<i64>,
    pub bucket_days: Vec<i32>,
}
impl Default for AgingQuery {
    fn default() -> Self {
        Self {
            through_entry_number: None,
            bucket_days: vec![30, 60, 90],
        }
    }
}
#[derive(Clone, Debug)]
pub struct AgedDocument {
    pub id: i64,
    pub document_number: i64,
    pub party_key: String,
    pub party_name: String,
    pub party_document_number: Option<String>,
    pub document_date: Date,
    pub due_date: Date,
    pub currency_code: String,
    pub total: Money,
    pub outstanding: Money,
    pub days_overdue: i32,
    pub bucket: usize,
}
#[derive(Clone, Debug)]
pub struct AgedParty {
    pub party_key: String,
    pub party_name: String,
    pub documents: Vec<AgedDocument>,
    pub buckets: Vec<Money>,
    pub total: Money,
}
#[derive(Clone, Debug)]
pub struct AgedPayables {
    pub company_id: i64,
    pub as_of: Date,
    pub through_entry_number: i64,
    pub bucket_days: Vec<i32>,
    pub parties: Vec<AgedParty>,
    pub buckets: Vec<Money>,
    pub total: Money,
    pub exceptions: Vec<AgedDocument>,
    pub exceptions_total: Money,
}
#[derive(Clone, Debug, Default)]
pub struct TaxQuery {
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub through_entry_number: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct TaxTotal {
    pub document_type: String,
    pub tax_code_id: i64,
    pub tax_code: String,
    pub rate: Decimal,
    pub account_id: i64,
    pub document_count: i64,
    pub taxable_amount: Money,
    pub tax_amount: Money,
}
#[derive(Clone, Debug)]
pub struct OtherPosting {
    pub owner_module: Option<String>,
    pub balance: Money,
}
#[derive(Clone, Debug)]
pub struct ControlAccount {
    pub account_id: i64,
    pub code: String,
    pub name: String,
    pub subledger_balance: Money,
    pub ledger_balance: Money,
    pub owned_balance: Money,
    pub other_postings: Vec<OtherPosting>,
    pub difference: Money,
}
#[derive(Clone, Debug)]
pub struct TieOut {
    pub company_id: i64,
    pub as_of: Date,
    pub through_entry_number: i64,
    pub currency_code: String,
    pub minor_units: MinorUnits,
    pub owner_module: String,
    pub accounts: Vec<ControlAccount>,
    pub total_difference: Money,
    pub tied_out: bool,
}

// Own-table sums only. Each reduction counts by both effective date and posting number.
const OWED: &str = r#"
SELECT b.id,b.total
- COALESCE((SELECT sum(n.total) FROM finance_payables_documents n WHERE n.tenant_id=b.tenant_id AND n.company_id=b.company_id AND n.corrects_document_id=b.id AND n.status='posted' AND n.document_date<=$3::text::date AND n.entry_number<=$4),0)
- COALESCE((SELECT sum(a.amount) FROM finance_payables_payment_allocations a JOIN finance_payables_payments p ON p.tenant_id=a.tenant_id AND p.company_id=a.company_id AND p.id=a.payment_id WHERE a.tenant_id=b.tenant_id AND a.company_id=b.company_id AND a.document_id=b.id AND p.payment_date<=$3::text::date AND p.entry_number<=$4 AND NOT EXISTS(SELECT 1 FROM finance_payables_payments r WHERE r.tenant_id=p.tenant_id AND r.company_id=p.company_id AND r.reverses_payment_id=p.id AND r.payment_date<=$3::text::date AND r.entry_number<=$4)),0) AS owed
FROM finance_payables_documents b WHERE b.tenant_id=$1 AND b.company_id=$2 AND b.document_type='bill' AND b.status='posted' AND b.document_date<=$3::text::date AND b.entry_number<=$4
"#;
pub(crate) async fn owed(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    bill: i64,
    date: Date,
    through: i64,
) -> bilibi_database::Result<Money> {
    let r = c
        .query_one(
            &format!("SELECT owed FROM ({OWED}) o WHERE id=$5"),
            &[&tenant, &company, &date.to_string(), &through, &bill],
        )
        .await?;
    store::money(r.get(0))
}
/// Minimum at the proposed date and every future balance-changing date.
pub(crate) async fn lowest(
    c: &impl GenericClient,
    tenant: i64,
    company: i64,
    bill: i64,
    since: Date,
) -> bilibi_database::Result<Money> {
    let dates=c.query("SELECT document_date::text FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND corrects_document_id=$3 AND status='posted' AND document_date>$4::text::date UNION SELECT p.payment_date::text FROM finance_payables_payments p JOIN finance_payables_payment_allocations a ON a.tenant_id=p.tenant_id AND a.company_id=p.company_id AND a.payment_id=COALESCE(p.reverses_payment_id,p.id) WHERE a.tenant_id=$1 AND a.company_id=$2 AND a.document_id=$3 AND p.payment_date>$4::text::date",&[&tenant,&company,&bill,&since.to_string()]).await?;
    let mut minimum = owed(c, tenant, company, bill, since, i64::MAX).await?;
    for d in dates {
        let date = d.get::<_, String>(0).parse().map_err(|_| Error::Invalid)?;
        minimum = minimum.min(owed(c, tenant, company, bill, date, i64::MAX).await?);
    }
    Ok(minimum)
}
fn fence(through: Option<i64>) -> Result<()> {
    if through.is_some_and(|n| n < 0) {
        Err(PayablesError::InvalidOption)
    } else {
        Ok(())
    }
}
impl Payables {
    pub async fn aged_payables(
        &self,
        s: &Scope,
        company: i64,
        as_of: Date,
        q: &AgingQuery,
    ) -> Result<AgedPayables> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        fence(q.through_entry_number)?;
        if q.bucket_days.is_empty()
            || q.bucket_days.iter().any(|v| *v <= 0)
            || q.bucket_days.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(PayablesError::InvalidOption);
        }
        let through=match q.through_entry_number {Some(n)=>n,None=>c.query_one("SELECT greatest(COALESCE((SELECT max(entry_number) FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2),0),COALESCE((SELECT max(entry_number) FROM finance_payables_payments WHERE tenant_id=$1 AND company_id=$2),0))",&[&s.tenant(),&company]).await?.get(0)};
        let rows=c.query(&format!("SELECT b.id,b.document_number,b.party_key,b.party_name,b.party_document_number,b.document_date::text,b.due_date::text,b.currency_code,b.total,o.owed FROM ({OWED}) o JOIN finance_payables_documents b ON b.id=o.id AND b.tenant_id=$1 AND b.company_id=$2 WHERE o.owed<>0 ORDER BY b.party_key,b.due_date,b.id"),&[&s.tenant(),&company,&as_of.to_string(),&through]).await?;
        let empty = vec![Money::ZERO; q.bucket_days.len() + 2];
        let mut parties: BTreeMap<String, AgedParty> = BTreeMap::new();
        let mut latest: BTreeMap<String, (Date, i64)> = BTreeMap::new();
        let mut exceptions = Vec::new();
        for r in rows {
            let due: Date = r.get::<_, String>(6).parse().map_err(|_| Error::Invalid)?;
            let days = due
                .until(as_of)
                .map_err(|_| Error::Invalid)?
                .get_days()
                .max(0);
            let bucket = if days == 0 {
                0
            } else {
                q.bucket_days
                    .iter()
                    .position(|v| days <= *v)
                    .map_or(q.bucket_days.len() + 1, |i| i + 1)
            };
            let d = AgedDocument {
                id: r.get(0),
                document_number: r.get(1),
                party_key: r.get(2),
                party_name: r.get(3),
                party_document_number: r.get(4),
                document_date: r.get::<_, String>(5).parse().map_err(|_| Error::Invalid)?,
                due_date: due,
                currency_code: r.get(7),
                total: store::money(r.get(8))?,
                outstanding: store::money(r.get(9))?,
                days_overdue: days,
                bucket,
            };
            if d.outstanding < Money::ZERO {
                exceptions.push(d);
                continue;
            }
            let party = parties
                .entry(d.party_key.clone())
                .or_insert_with(|| AgedParty {
                    party_key: d.party_key.clone(),
                    party_name: d.party_name.clone(),
                    documents: vec![],
                    buckets: empty.clone(),
                    total: Money::ZERO,
                });
            let date = (d.document_date, d.id);
            if latest.get(&d.party_key).is_none_or(|v| date > *v) {
                party.party_name = d.party_name.clone();
                latest.insert(d.party_key.clone(), date);
            }
            party.buckets[bucket] = party.buckets[bucket] + d.outstanding;
            party.total = party.total + d.outstanding;
            party.documents.push(d);
        }
        let parties: Vec<_> = parties.into_values().collect();
        let mut buckets = empty;
        for p in &parties {
            for (i, a) in p.buckets.iter().enumerate() {
                buckets[i] = buckets[i] + *a;
            }
        }
        Ok(AgedPayables {
            company_id: company,
            as_of,
            through_entry_number: through,
            bucket_days: q.bucket_days.clone(),
            total: parties.iter().map(|p| p.total).sum(),
            parties,
            buckets,
            exceptions_total: exceptions.iter().map(|e| e.outstanding).sum(),
            exceptions,
        })
    }
    pub async fn tax_totals(&self, s: &Scope, company: i64, q: &TaxQuery) -> Result<Vec<TaxTotal>> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        fence(q.through_entry_number)?;
        if q.from.zip(q.to).is_some_and(|(f, t)| f > t) {
            return Err(PayablesError::InvalidOption);
        }
        let rows=c.query("SELECT d.document_type,t.tax_code_id,t.tax_code,t.rate,t.account_id,count(DISTINCT d.id),sum(t.taxable_amount),sum(t.tax_amount) FROM finance_payables_document_tax_lines t JOIN finance_payables_documents d ON d.id=t.document_id AND d.tenant_id=t.tenant_id AND d.company_id=t.company_id WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.status='posted' AND ($3::text IS NULL OR d.document_date>=$3::text::date) AND ($4::text IS NULL OR d.document_date<=$4::text::date) AND ($5::bigint IS NULL OR d.entry_number<=$5) GROUP BY d.document_type,t.tax_code_id,t.tax_code,t.rate,t.account_id ORDER BY t.tax_code,d.document_type,t.rate,t.account_id",&[&s.tenant(),&company,&q.from.map(|d|d.to_string()),&q.to.map(|d|d.to_string()),&q.through_entry_number]).await?;
        rows.iter()
            .map(|r| {
                Ok(TaxTotal {
                    document_type: r.get(0),
                    tax_code_id: r.get(1),
                    tax_code: r.get(2),
                    rate: r.get(3),
                    account_id: r.get(4),
                    document_count: r.get(5),
                    taxable_amount: store::money(r.get(6))?,
                    tax_amount: store::money(r.get(7))?,
                })
            })
            .collect()
    }
    pub async fn tie_out_control_accounts(
        &self,
        s: &Scope,
        company: i64,
        as_of: Date,
        through: Option<i64>,
    ) -> Result<TieOut> {
        // Release this connection before reading the ledger through its facade.
        {
            let c = self.db.get().await.map_err(Error::from)?;
            authorize(&**c, s, company, VIEW).await?;
        }
        fence(through)?;
        let ledger = self
            .ledger
            .account_totals_by_owner(
                s,
                company,
                &TotalsQuery {
                    to: Some(as_of),
                    through_entry_number: through,
                    ..Default::default()
                },
            )
            .await?;
        let chart = self.ledger.list_accounts(s, company).await?;
        let c = self.db.get().await.map_err(Error::from)?;
        let rows=c.query(&format!("SELECT b.payable_account_id,sum(o.owed) FROM ({OWED}) o JOIN finance_payables_documents b ON b.id=o.id AND b.tenant_id=$1 AND b.company_id=$2 GROUP BY b.payable_account_id"),&[&s.tenant(),&company,&as_of.to_string(),&ledger.through_entry_number]).await?;
        let mut subledger = BTreeMap::new();
        for r in rows {
            subledger.insert(r.get::<_, i64>(0), store::money(r.get(1))?);
        }
        let ids=c.query("SELECT payable_account_id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND payable_account_id IS NOT NULL UNION SELECT payable_account_id FROM finance_payables_settings WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?;
        let mut accounts = Vec::new();
        for r in ids {
            let id: i64 = r.get(0);
            let a = chart.iter().find(|a| a.id == id).ok_or(Error::Invalid)?;
            let postings: Vec<_> = ledger.rows.iter().filter(|p| p.account.id == id).collect();
            let balance = postings.iter().map(|p| p.credits - p.debits).sum();
            let owned = postings
                .iter()
                .filter(|p| p.owner_module.as_deref() == Some(MODULE))
                .map(|p| p.credits - p.debits)
                .sum();
            let sub = subledger.get(&id).copied().unwrap_or(Money::ZERO);
            let other_postings = postings
                .iter()
                .filter(|p| p.owner_module.as_deref() != Some(MODULE))
                .map(|p| OtherPosting {
                    owner_module: p.owner_module.clone(),
                    balance: p.credits - p.debits,
                })
                .collect();
            accounts.push(ControlAccount {
                account_id: id,
                code: a.code.clone(),
                name: a.name.clone(),
                subledger_balance: sub,
                ledger_balance: balance,
                owned_balance: owned,
                other_postings,
                difference: sub - balance,
            });
        }
        accounts.sort_by(|a, b| a.code.cmp(&b.code));
        Ok(TieOut {
            company_id: company,
            as_of,
            through_entry_number: ledger.through_entry_number,
            currency_code: ledger.currency_code,
            minor_units: ledger.minor_units,
            owner_module: MODULE.into(),
            total_difference: accounts.iter().map(|a| a.difference).sum(),
            tied_out: accounts.iter().all(|a| a.difference.is_zero()),
            accounts,
        })
    }
}
