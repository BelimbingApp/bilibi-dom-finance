//! Bilimbi.Finance.FixedAssets register: the company lock serializes acquisition
//! with source reversal and duplicate registration. Sources keep their owners.
use crate::*;
use bilibi_finance_ledger::Entry;
use jiff::ToSpan;
use serde_json::json;
use store::Table;

impl FixedAssets {
    pub async fn acquire_from_bill(
        &self,
        s: &Scope,
        company: i64,
        bill_id: i64,
        bill_line_number: i32,
        input: &AcquisitionInput,
    ) -> Result<Asset> {
        self.acquire(s, company, bill_id, bill_line_number, true, input)
            .await
    }
    /// As in Bilimbi, this path accepts any live original acquisition debit,
    /// not only ownerless entries. It does not infer optional bill provenance.
    pub async fn acquire_from_entry(
        &self,
        s: &Scope,
        company: i64,
        entry_id: i64,
        entry_line_number: i32,
        input: &AcquisitionInput,
    ) -> Result<Asset> {
        self.acquire(s, company, entry_id, entry_line_number, false, input)
            .await
    }
    async fn acquire(
        &self,
        s: &Scope,
        company: i64,
        source_id: i64,
        line_number: i32,
        bill: bool,
        input: &AcquisitionInput,
    ) -> Result<Asset> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(&mut c, s, company, ACQUIRE, "finance.fixed-assets.acquire")
            .await?;
        if source_id <= 0 {
            return Err(invalid(
                if bill { "bill_id" } else { "entry_id" },
                "must be greater than 0",
            ));
        }
        if line_number <= 0 {
            return Err(invalid(
                if bill {
                    "bill_line_number"
                } else {
                    "entry_line_number"
                },
                "must be greater than 0",
            ));
        }
        let entry_id = if bill {
            let doc = self
                .payables
                .get_document_on(tx.sql(), s, company, source_id)
                .await?;
            if doc.document_type != "bill" || doc.status != "posted" {
                return Err(FixedAssetsError::BillNotPosted);
            }
            if !doc.lines.iter().any(|l| l.line_number == line_number) {
                return Err(FixedAssetsError::BillLineNotFound);
            }
            doc.entry_id.ok_or(Error::Invalid)?
        } else {
            source_id
        };
        let entry = self
            .ledger
            .get_entry_on(tx.sql(), s, company, entry_id)
            .await?;
        if let Ok(opening) = self.ledger.opening_context_on(tx.sql(), s, company).await {
            if entry.id == opening.entry.id {
                return Err(invalid(
                    "entry_id",
                    "is an opening balance; import the opening asset register instead of registering its aggregate debit",
                ));
            }
        }
        let revision =
            store::revision(tx.sql(), s.tenant(), company, input.category_revision_id).await?;
        let book = self.ledger.get_book_on(tx.sql(), s, company).await?;
        let cost = acquisition_debit(&entry, line_number, revision.defaults.cost_account_id)?;
        if tx.sql().query_opt("SELECT id FROM finance_fixed_assets_assets WHERE tenant_id=$1 AND company_id=$2 AND entry_id=$3 AND entry_line_number=$4",&[&s.tenant(),&company,&entry_id,&line_number]).await?.is_some() {return Err(invalid("entry_id","has already been taken"));}
        let description = text(&input.description, "description", 500)?;
        if input.in_service_date < entry.entry_date {
            return Err(invalid("in_service_date", "cannot precede acquisition"));
        }
        let residual = Money::new(
            input.residual_value.unwrap_or(Money::ZERO).amount(),
            book.minor_units,
        )
        .map_err(|e| invalid("residual_value", e.to_string()))?;
        if residual < Money::ZERO {
            return Err(invalid(
                "residual_value",
                "must be greater than or equal to 0",
            ));
        }
        if residual > cost {
            return Err(invalid(
                "residual_value",
                format!("must be less than or equal to {cost}"),
            ));
        }
        let a = schedule::assumptions(Assumptions {
            method: input.method.unwrap_or(revision.defaults.assumptions.method),
            life_months: input
                .life_months
                .unwrap_or(revision.defaults.assumptions.life_months),
            annual_rate: input
                .annual_rate
                .or(revision.defaults.assumptions.annual_rate),
        })?;
        input
            .in_service_date
            .first_of_month()
            .checked_add((a.life_months - 1).months())
            .map_err(|_| invalid("in_service_date", "is invalid"))?;
        let mut row = store::identity(s, company);
        let values = json!({"category_revision_id":revision.id,"description":description,"acquisition_date":entry.entry_date.to_string(),"in_service_date":input.in_service_date.to_string(),"cost":cost,"residual_value":residual,"method":a.method,"life_months":a.life_months,"annual_rate":a.annual_rate,"entry_id":entry.id,"entry_number":entry.entry_number,"entry_line_number":line_number,"bill_id":bill.then_some(source_id),"bill_line_number":bill.then_some(line_number)});
        row.as_object_mut()
            .ok_or(Error::Invalid)?
            .extend(values.as_object().ok_or(Error::Invalid)?.clone());
        let value = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let (v, change) = store::insert(sql, Table::Asset, row).await?;
                    Ok((v, vec![change]))
                })
            })
            .await?;
        let asset = store::asset(&value, book.minor_units)?;
        tx.commit().await?;
        Ok(asset)
    }
    pub async fn get_asset(&self, s: &Scope, company: i64, id: i64) -> Result<AssetDetail> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let tx = c
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await?;
        authorize(&*tx, s, company, VIEW).await?;
        let v = store::raw(&*tx, Table::Asset, s.tenant(), company, id)
            .await?
            .ok_or(FixedAssetsError::NotFound)?;
        let book = self.ledger.get_book_on(&*tx, s, company).await?;
        let asset = store::asset(&v, book.minor_units)?;
        let revision =
            store::revision(&*tx, s.tenant(), company, asset.category_revision_id).await?;
        let schedule = schedule::rows(&asset, book.minor_units)?;
        let activity = activity::Activity::load(&*tx, s, company, book.minor_units).await?;
        let voided = activity.voided(id);
        let events = activity
            .events
            .iter()
            .filter(|e| e.asset_id == Some(id))
            .cloned()
            .collect();
        let mut charges = Vec::new();
        for run in &activity.runs {
            for item in run.items.iter().filter(|i| i.asset_id == id) {
                let reversal = activity
                    .events
                    .iter()
                    .filter(|e| {
                        (e.run_id == Some(run.id) && e.action == EventAction::RunReversed)
                            || (e.asset_id == Some(id)
                                && e.action == EventAction::AcquisitionVoided)
                    })
                    .min_by_key(|e| e.event_number)
                    .cloned();
                charges.push(AssetCharge {
                    run_id: run.id,
                    period: run.period,
                    amount: item.amount,
                    entry_number: run.entry_number,
                    recorded_by_user_id: run.recorded_by_user_id,
                    recorded_by_impersonator_id: run.recorded_by_impersonator_id,
                    recorded_at: run.recorded_at.clone(),
                    reversal,
                });
            }
        }
        let opening=tx.query_opt("SELECT i.source_facts || jsonb_build_object('source_reference',b.source_reference) FROM finance_fixed_assets_opening_items i JOIN finance_fixed_assets_opening_batches b ON b.id=i.batch_id AND b.tenant_id=$1 AND b.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.asset_id=$3",&[&s.tenant(),&company,&id]).await?.map(|r|r.get(0));
        tx.commit().await?;
        Ok(AssetDetail {
            opening,
            asset,
            revision,
            schedule,
            charges,
            events,
            voided,
        })
    }
}
fn acquisition_debit(entry: &Entry, number: i32, cost_account: i64) -> Result<Money> {
    if entry.reversed_by_entry_id.is_some() || entry.reverses_entry_id.is_some() {
        return Err(FixedAssetsError::AcquisitionReversed);
    }
    let line = entry
        .lines
        .iter()
        .find(|l| l.line_number == number)
        .ok_or(FixedAssetsError::EntryLineNotFound)?;
    if line.account_id != cost_account || !line.debit.is_positive() || !line.credit.is_zero() {
        return Err(FixedAssetsError::NotAcquisitionDebit);
    }
    Ok(line.debit)
}
