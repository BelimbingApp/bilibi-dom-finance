//! Domain-owned conversion evidence. Compose the owners, freeze their answers;
//! never infer a clean conversion merely because the trial balance balances.
use crate::{
    authorize, issued, json, pack_owner, Reports, ReportsError, Result, ISSUE, MODULE, VIEW,
};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_finance_ledger::{Approval, Money, Preparer, TotalsQuery};
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_postgres::{GenericClient, Transaction};

pub(crate) const KIND: &str = "conversion";
const MAX_BYTES: usize = 32 * 1024 * 1024;

impl Reports {
    /// A coherent conversion preview. Issuance collects the same facts on the
    /// reviewed write's transaction, then stores them rather than just pins.
    pub async fn conversion_report(
        &self,
        scope: &Scope,
        company: i64,
        as_of: Date,
    ) -> Result<Value> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let report = self.conversion_on(&tx, scope, company, as_of).await?;
        tx.commit().await?;
        Ok(report)
    }

    pub(crate) async fn conversion_access(
        &self,
        c: &(impl GenericClient + Sync),
        s: &Scope,
        company: i64,
        capability: &str,
    ) -> Result<()> {
        authorize(c, s, company, capability).await?;
        pack_owner::authorize_sources(
            c,
            s,
            OrganisationId::new(company).map_err(|_| ReportsError::CompanyNotFound)?,
        )
        .await
        .map_err(crate::refusal)
    }

    pub(crate) async fn conversion_on(
        &self,
        c: &Transaction<'_>,
        s: &Scope,
        company: i64,
        as_of: Date,
    ) -> Result<Value> {
        self.conversion_access(c, s, company, VIEW).await?;
        let opening = self.ledger.opening_context_on(c, s, company).await?;
        if as_of < opening.entry.entry_date {
            return Err(ReportsError::InvalidOption);
        }
        let target = self
            .ledger
            .trial_balance_on(c, s, company, as_of, None)
            .await?;
        let through = target.through_entry_number;

        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        let query = TotalsQuery {
            to: Some(as_of),
            through_entry_number: Some(through),
            ..Default::default()
        };
        let owners = self
            .ledger
            .account_totals_by_owner_on(c, s, company, &query)
            .await?;
        let ar = self
            .sources
            .receivables
            .tie_out_control_accounts_on(
                c,
                s,
                company,
                as_of,
                &bilibi_finance_receivables::BalancePins {
                    through_entry_number: Some(through),
                    ..Default::default()
                },
            )
            .await
            .map_err(source_error)?;
        let ap = self
            .sources
            .payables
            .tie_out_control_accounts_on(
                c,
                s,
                company,
                as_of,
                &bilibi_finance_payables::BalancePins {
                    through_entry_number: Some(through),
                    ..Default::default()
                },
            )
            .await
            .map_err(source_error)?;
        let ar_detail = self
            .sources
            .receivables
            .aged_receivables_on(
                c,
                s,
                company,
                as_of,
                &bilibi_finance_receivables::AgingOptions {
                    through_entry_number: Some(through),
                    through_opening_batch_id: Some(ar.through_opening_batch_id),
                    ..Default::default()
                },
            )
            .await
            .map_err(source_error)?;
        let ap_detail = self
            .sources
            .payables
            .aged_payables_on(
                c,
                s,
                company,
                as_of,
                &bilibi_finance_payables::AgingQuery {
                    through_entry_number: Some(through),
                    through_opening_batch_id: Some(ap.through_opening_batch_id),
                    ..Default::default()
                },
            )
            .await
            .map_err(source_error)?;
        let assets = self
            .sources
            .fixed_assets
            .register_report_on(
                c,
                s,
                company,
                as_of,
                &bilibi_finance_fixed_assets::ReportPins {
                    through_entry_number: Some(through),
                    ..Default::default()
                },
            )
            .await
            .map_err(source_error)?;
        let comparisons: Vec<_> = chart.iter().filter(|a| !a.is_group).map(|a| {
            let source_balance=opening.balances.get(&a.id).copied().unwrap_or(Money::ZERO);
            let target_balance=target.rows.iter().find(|r|r.account.id==a.id).map(|r|r.debit_balance-r.credit_balance).unwrap_or(Money::ZERO);
            json!({"account_id":a.id,"code":a.code,"name":a.name,"source_balance":source_balance,"source_debit_balance":source_balance.max(Money::ZERO),"source_credit_balance":(-source_balance).max(Money::ZERO),"target_balance":target_balance,"movement":target_balance-source_balance})
        }).collect();
        let mut evidence = serde_json::Map::new();
        // Each source owns both authorization and its frozen explicit field lists.
        // Never silently cut a register at one evidence page.
        macro_rules! retain {
            ($key:literal,$owner:expr,$datasets:expr) => {{
                let mut datasets=serde_json::Map::new();
                for dataset in $datasets {
                    let mut after=None; let mut rows=Vec::new(); let mut bytes=0usize;
                    let fields=loop {
                        let page=$owner.evidence_page_on(c,s,company,dataset,Some(500),after).await.map_err(source_error)?;
                        bytes+=serde_json::to_vec(&page.rows).map_err(|_|Error::Internal)?.len();
                        if bytes>MAX_BYTES {return Err(ReportsError::InvalidOption);}
                        rows.extend(page.rows);
                        if page.next.is_none() {break page.fields;}
                        after=page.next;
                    };
                    datasets.insert(dataset.to_string(),json!({"fields":fields,"rows":rows,"through_id":rows.last().and_then(|r|r["id"].as_i64()).unwrap_or(0),"sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&rows).map_err(|_|Error::Internal)?))}));
                }
                evidence.insert($key.into(),Value::Object(datasets));
            }};
        }
        retain!(
            "ledger",
            self.ledger,
            [
                "opening_balances",
                "opening_recognition",
                "opening_recognition_replacement",
                "journal_event",
                "closing_event"
            ]
        );
        retain!(
            "receivables",
            self.sources.receivables,
            ["opening_batch", "opening_item", "opening_decision"]
        );
        retain!(
            "payables",
            self.sources.payables,
            ["opening_batch", "opening_item", "opening_decision"]
        );
        retain!(
            "fixed_assets",
            self.sources.fixed_assets,
            ["opening_batch", "opening_item", "opening_decision"]
        );
        retain!(
            "bank",
            self.sources.reconciliation,
            [
                "bank_account",
                "statement",
                "reconciliation_record",
                "reconciliation_event",
                "reconciliation_item"
            ]
        );
        retain!(
            "tax",
            self.sources.sst,
            bilibi_finance_sst::Sst::evidence_datasets()
        );
        retain!(
            "payroll",
            self.sources.payroll,
            bilibi_finance_payroll::Payroll::evidence_datasets()
        );
        let mut bank_bridges = Vec::new();
        let bank_accounts = evidence["bank"]["bank_account"]["rows"]
            .as_array()
            .ok_or(Error::Internal)?;
        let bank_records = evidence["bank"]["reconciliation_record"]["rows"]
            .as_array()
            .ok_or(Error::Internal)?;
        for account in bank_accounts {
            let bank_id = account["id"].as_i64().ok_or(Error::Internal)?;
            let ledger_id = account["ledger_account_id"]
                .as_i64()
                .ok_or(Error::Internal)?;
            let records = bank_records.iter().filter(|r| {
                r["bank_account_id"].as_i64() == Some(bank_id)
                    && r["end_date"]
                        .as_str()
                        .is_some_and(|d| d <= as_of.to_string().as_str())
            });
            let mut bridge = None;
            for record in records {
                let read = self
                    .sources
                    .reconciliation
                    .get_reconciliation_on(
                        c,
                        s,
                        company,
                        record["id"].as_i64().ok_or(Error::Internal)?,
                    )
                    .await
                    .map_err(source_error)?;
                if read.status != bilibi_finance_reconciliation::ReconciliationStatus::Abandoned
                    && bridge.as_ref().is_none_or(
                        |b: &bilibi_finance_reconciliation::BankReconciliation| {
                            (b.end_date, b.id) < (read.end_date, read.id)
                        },
                    )
                {
                    bridge = Some(read);
                }
            }
            let current_balance = target
                .rows
                .iter()
                .find(|r| r.account.id == ledger_id)
                .map(|r| r.debit_balance - r.credit_balance)
                .unwrap_or(Money::ZERO);
            let (end_balance, changed) = if let Some(bridge) = &bridge {
                let totals = self
                    .ledger
                    .account_totals_on(
                        c,
                        s,
                        company,
                        &TotalsQuery {
                            to: Some(bridge.end_date),
                            through_entry_number: Some(through),
                            ..Default::default()
                        },
                    )
                    .await?;
                let end = totals
                    .rows
                    .iter()
                    .find(|r| r.account.id == ledger_id)
                    .map(|r| r.debits - r.credits)
                    .unwrap_or(Money::ZERO);
                (
                    Some(end),
                    bridge.figures.as_ref().map(|f| end - f.book_balance),
                )
            } else {
                (None, None)
            };
            bank_bridges.push(json!({"account":account,"target_balance":current_balance,"bridge_end_balance":end_balance,"change_since_bridge":changed,"bridge":bridge}));
        }
        let mut differences = Vec::new();
        for row in &ar.accounts {
            if !row.difference.is_zero() {
                differences.push(json!({"area":"receivables","account_id":row.account_id,"difference":row.difference}));
            }
        }
        for row in &ap.accounts {
            if row.difference != Money::ZERO {
                differences.push(json!({"area":"payables","account_id":row.account_id,"difference":row.difference}));
            }
        }
        for row in &assets.reconciliation {
            if row.difference != Money::ZERO {
                differences.push(json!({"area":"fixed_assets","account_id":row.account_id,"kind":row.kind,"difference":row.difference}));
            }
        }
        for bank in &bank_bridges {
            let incomplete = bank["bridge"].is_null()
                || bank["bridge"]["status"] != "finalised"
                || bank["bridge"]["end_date"].as_str() != Some(as_of.to_string().as_str());
            let changed = bank["change_since_bridge"]
                .as_str()
                .and_then(|v| Money::parse(v, target.minor_units).ok())
                .is_some_and(|m| m != Money::ZERO);
            if incomplete || changed {
                differences.push(json!({"area":"bank","bank_account_id":bank["account"]["id"],"reason":"Review the bridge's status, end date and changes since finalisation; it does not establish a complete tie-out at the report date."}));
            }
        }
        let report = json!({"version":1,"company_id":company,"as_of":as_of.to_string(),"conversion_date":opening.entry.entry_date.to_string(),"currency_code":target.currency_code,"minor_units":target.minor_units,"through_entry_number":through,
            "opening":opening,"source_trial_balance":{"rows":comparisons,"basis":"Approved source opening journal only; signed debit less credit balances."},"target_trial_balance":target,"accounts":comparisons,"postings_by_owner":owners,
            "receivables":{"detail":ar_detail,"control":ar},"payables":{"detail":ap_detail,"control":ap},"assets":assets,"bank_bridges":bank_bridges,"sources":evidence,
            "unresolved_differences":differences,
            "excluded_history":["Transactions before conversion other than the approved opening journal and retained opening registers.","Previously settled invoice and bill history, historical asset depreciation entries and legacy bank matches are not imported."],
            "schedule_limits":["Bank evidence retains statement and finalised reconciliation bridges. Open bridges require a separate reconciliation; no conversion bank bridge is inferred.","Tax and payroll evidence is retained where recorded. Legacy returns, payroll year-to-date and filing completeness require the accountant's external schedules and reviewer references."],
            "assessment":"Unassessed. Zero numerical differences do not establish completeness or accountant approval. Other-owner control postings remain visible; opening recognition is evidence, not a reclassification."});
        if serde_json::to_vec(&report)
            .map_err(|_| Error::Internal)?
            .len()
            > MAX_BYTES
        {
            return Err(ReportsError::InvalidOption);
        }
        Ok(report)
    }

    pub(crate) async fn issue_conversion_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        content: Value,
    ) -> Result<Value> {
        self.conversion_access(tx.sql(), s, company, ISSUE).await?;
        let new = issued::NewIssued {
            tenant_id: s.tenant(),
            company_id: company,
            kind: KIND,
            implementation_version: "conversion-1",
            through_entry_number: content["through_entry_number"]
                .as_i64()
                .ok_or(Error::Internal)?,
            chart: content["target_trial_balance"]["rows"].clone(),
            layout: json!({}),
            membership: json!({}),
            content,
            issued_by_user_id: s.user(),
            issued_by_impersonator_id: s.impersonator().map(|i| i.user()),
        };
        let row = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let row = issued::insert(sql, &new).await?;
                    let change = Change {
                        kind: "finance.reports.issued".into(),
                        id: row.id,
                        old: Value::Null,
                        new: issued::summary(&row),
                    };
                    Ok((row, vec![change]))
                })
            })
            .await?;
        json(&row)
    }

    pub(crate) async fn conversion_issued_on(
        &self,
        c: &(impl GenericClient + Sync),
        s: &Scope,
        company: i64,
        id: i64,
    ) -> Result<Value> {
        self.conversion_access(c, s, company, VIEW).await?;
        let row = issued::find(c, s.tenant(), company, id)
            .await?
            .filter(|r| r.kind == KIND)
            .ok_or(ReportsError::ReportNotFound)?;
        let signs=c.query("SELECT to_jsonb(s) FROM finance_reports_conversion_signoffs s WHERE tenant_id=$1 AND company_id=$2 AND report_id=$3 ORDER BY id",&[&s.tenant(),&company,&id]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>();
        Ok(json!({"report":row,"signoffs":signs}))
    }

    pub(crate) fn conversion_approval(report: &Value, reason: Option<String>) -> Result<Approval> {
        let row = &report["report"];
        let mut prepared = vec![Preparer {
            user_id: row["issued_by_user_id"].as_i64().ok_or(Error::Internal)?,
            impersonator_id: row["issued_by_impersonator_id"].as_i64(),
        }];
        let opening = &row["content"]["opening"]["prepared_by"];
        prepared.push(Preparer {
            user_id: opening["user_id"].as_i64().ok_or(Error::Internal)?,
            impersonator_id: opening["impersonator_id"].as_i64(),
        });
        for module in ["receivables", "payables", "fixed_assets"] {
            if let Some(rows) =
                row["content"]["sources"][module]["opening_batch"]["rows"].as_array()
            {
                for r in rows {
                    prepared.push(Preparer {
                        user_id: r["recorded_by_user_id"].as_i64().ok_or(Error::Internal)?,
                        impersonator_id: r["recorded_by_impersonator_id"].as_i64(),
                    });
                }
            }
        }
        Ok(Approval {
            source: MODULE.into(),
            action: "conversion_signoff".into(),
            subject_type: "issued_report".into(),
            subject_id: row["id"].as_i64().ok_or(Error::Internal)?,
            subject: "Conversion report sign-off".into(),
            prepared_by: prepared,
            override_reason: reason,
        })
    }
}
fn source_error<E: std::fmt::Display>(error: E) -> ReportsError {
    ReportsError::Ledger(bilibi_finance_ledger::LedgerError::Invalid(vec![
        bilibi_finance_ledger::FieldError::new("base", error.to_string()),
    ]))
}
