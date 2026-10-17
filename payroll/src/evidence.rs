//! Explicit Payroll export fields, mirroring Bilimbi Payroll.Evidence.
use crate::{authorize, Payroll, PayrollError, Result, VIEW};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use serde::Serialize;
use serde_json::Value;
pub(crate) const DATASETS: &[&str] = &[
    "mapping",
    "mapping_change",
    "posting_event",
    "settings",
    "settings_change",
];

#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}

fn dataset(name: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match name {
        "mapping" => (
            "finance_payroll_mappings",
            &[
                "id",
                "tenant_id",
                "company_id",
                "kind",
                "code",
                "direction",
                "expense_account_id",
                "liability_account_id",
                "inserted_at",
                "updated_at",
            ],
        ),
        "mapping_change" => (
            "finance_payroll_mapping_changes",
            &[
                "id",
                "tenant_id",
                "company_id",
                "mapping_id",
                "old_expense_account_id",
                "new_expense_account_id",
                "old_liability_account_id",
                "new_liability_account_id",
                "reason",
                "changed_by_user_id",
                "changed_by_impersonator_id",
                "changed_at",
            ],
        ),
        "settings" => (
            "finance_payroll_settings",
            &[
                "id",
                "tenant_id",
                "company_id",
                "net_pay_account_id",
                "claims_payable_account_id",
                "inserted_at",
                "updated_at",
            ],
        ),
        "settings_change" => (
            "finance_payroll_settings_changes",
            &[
                "id",
                "tenant_id",
                "company_id",
                "old_net_pay_account_id",
                "new_net_pay_account_id",
                "old_claims_payable_account_id",
                "new_claims_payable_account_id",
                "reason",
                "changed_by_user_id",
                "changed_by_impersonator_id",
                "changed_at",
            ],
        ),
        "posting_event" => (
            "finance_payroll_posting_events",
            &[
                "id",
                "tenant_id",
                "company_id",
                "source_kind",
                "source_id",
                "source_reference",
                "event",
                "entry_id",
                "entry_number",
                "entry_date",
                "total",
                "currency_code",
                "reason",
                "actor_user_id",
                "actor_impersonator_id",
                "occurred_at",
            ],
        ),
        _ => return None,
    })
}
impl Payroll {
    /// ID-ordered, company-scoped evidence. A full page returns its last ID;
    /// an empty follow-up can therefore terminate a full final page.
    pub async fn evidence_page(
        &self,
        scope: &Scope,
        company: i64,
        name: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company, VIEW).await?;
        let (table, fields) = dataset(name).ok_or(PayrollError::InvalidOption)?;
        let (limit, after) = (limit.unwrap_or(500), after.unwrap_or(0));
        if !(1..=500).contains(&limit) || after < 0 {
            return Err(PayrollError::InvalidOption);
        }
        // Only the allowlisted identifiers above enter SQL; all data is bound.
        // Numeric totals are strings so a downstream JSON reader cannot lose cents.
        let columns = fields
            .iter()
            .map(|f| {
                if *f == "total" {
                    "total::text AS total".to_owned()
                } else if matches!(
                    *f,
                    "changed_at" | "occurred_at" | "inserted_at" | "updated_at"
                ) {
                    format!("to_char({f},'{}') AS {f}", crate::store::ISO)
                } else {
                    (*f).to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let rows = client.query(&format!("SELECT to_jsonb(r) FROM (SELECT {columns} FROM {table} WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4) r"), &[&scope.tenant(), &company, &after, &limit]).await?.iter().map(|r| r.get(0)).collect::<Vec<Value>>();
        let next = (rows.len() as i64 == limit)
            .then(|| rows.last().and_then(|r| r["id"].as_i64()))
            .flatten();
        Ok(EvidencePage {
            fields: fields.to_vec(),
            rows,
            next,
        })
    }
}
