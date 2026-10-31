//! Source register state is a prospective opening estimate. Historical cost,
//! dates and accumulation remain evidence; future runs start at conversion.
use crate::*;
use bilibi_agent_api::OperationClient;
use bilibi_finance_ledger::{EntryOwner, OpeningContext};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
pub(super) const CONVENTION: &str = "Source balances are frozen immediately before the conversion date. Future depreciation starts in the conversion calendar month, including a mid-month cut-off; a full first month is recognized at month-end. No daily proration or historical catch-up charge is inferred.";
pub(super) const HEADER: &[&str] = &[
    "source_id",
    "description",
    "category_revision_id",
    "acquisition_date",
    "in_service_date",
    "cost",
    "accumulated_depreciation",
    "remaining_life_months",
    "residual_value",
];
pub(super) struct Prepared {
    pub review: Value,
    context: OpeningContext,
    rows: Vec<Value>,
    balances: BTreeMap<i64, Money>,
    existing: Option<Value>,
}
impl FixedAssets {
    pub(super) async fn opening_authorize(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        cap: &str,
    ) -> Result<()> {
        bilibi_company::authorize_work_target(c, s, company, cap)
            .await
            .map(|_| ())
            .map_err(|e| match e {
                Error::Forbidden | Error::Unauthorized => FixedAssetsError::Unauthorized,
                Error::NotFound => FixedAssetsError::CompanyNotFound,
                e => FixedAssetsError::Failed(e),
            })
    }
    pub(super) async fn opening_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.opening_authorize(c, s, company, VIEW).await?;
        let after = input["items_after"].as_i64().unwrap_or(0);
        let limit = input["limit"].as_i64().unwrap_or(200).clamp(1, 500);
        let items: Vec<Value> = c.query("SELECT to_jsonb(i) FROM finance_fixed_assets_opening_items i WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4", &[&s.tenant(), &company, &after, &(limit + 1)]).await?.iter().map(|row| row.get(0)).collect();
        let more = items.len() as i64 > limit;
        let items: Vec<Value> = items.into_iter().take(limit as usize).collect();
        let next = if more {
            items.last().map(|i| i["id"].clone())
        } else {
            None
        };
        let total: i64 = c.query_opt("SELECT count(*) FROM finance_fixed_assets_opening_items WHERE tenant_id=$1 AND company_id=$2", &[&s.tenant(), &company]).await?.ok_or(Error::Internal)?.get(0);
        let categories:Vec<Value>=c.query("SELECT to_jsonb(r) || jsonb_build_object('category_name',g.name) FROM finance_fixed_assets_category_revisions r JOIN finance_fixed_assets_categories g ON g.id=r.category_id AND g.tenant_id=r.tenant_id AND g.company_id=r.company_id WHERE r.tenant_id=$1 AND r.company_id=$2 ORDER BY r.id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get(0)).collect();
        Ok(
            json!({"opening":self.ledger.opening_context_on(c,s,company).await?,"book":self.ledger.get_book_on(c,s,company).await?,"batch":c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_fixed_assets_opening_batches b WHERE tenant_id=$1 AND company_id=$2 ORDER BY id DESC LIMIT 1",&[&s.tenant(),&company]).await?.map(|r|r.get::<_,Value>(0)),"category_revisions":categories,"stages":c.query("SELECT to_jsonb(s)-'payload'||jsonb_build_object('part_count',payload->'part_count','csv_bytes',octet_length(payload->>'csv')) FROM finance_fixed_assets_opening_stages s WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"decisions":c.query("SELECT to_jsonb(d) FROM finance_fixed_assets_opening_decisions d WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"month_convention":CONVENTION,"header":HEADER,"items":items,"item_count":total,"items_next":next}),
        )
    }
    pub(super) async fn prepare_opening(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Prepared> {
        self.opening_authorize(c, s, company, ACQUIRE).await?;
        self.lifecycle_preview_lock(c, s, company).await?;
        let context = self.ledger.opening_context_on(c, s, company).await?;
        let book = self.ledger.get_book_on(c, s, company).await?;
        let reference = input["source_reference"].as_str().unwrap_or("").trim();
        if reference.is_empty() || reference.chars().count() > 100 || reference.contains('\0') {
            return Err(invalid(
                "source_reference",
                "must contain 1 to 100 characters without null characters",
            ));
        }
        let csv = input["csv"]
            .as_str()
            .ok_or_else(|| invalid("csv", "is required"))?;
        if csv.contains('\0') {
            return Err(invalid("csv", "must not contain null characters"));
        }
        if input["currency_code"] != book.currency_code {
            return Err(invalid("currency_code", "must match the book's currency"));
        }
        if let Some(row)=c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_fixed_assets_opening_batches b WHERE tenant_id=$1 AND company_id=$2 AND source_reference=$3",&[&s.tenant(),&company,&reference]).await? {
            let batch:Value=row.get(0);
            if batch["source_reference"]!=reference || batch["source_csv"]!=csv {return Err(invalid("source_reference","the opening register has already been imported; the same source reference and exact CSV are required for a retry"));}
            let retained=c.query_opt("SELECT review FROM finance_fixed_assets_opening_decisions WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&batch["id"].as_i64().ok_or(Error::Invalid)?]).await?;
            if let Some(row)=retained {let review:Value=row.get(0);if review["replaces_batch_id"]!=input["replaces_batch_id"] || review["reason"]!=input["reason"] {return Err(invalid("source_reference","already identifies different correction instructions"));}}
            return Ok(Prepared{review:json!({"state":"reconciled","batch":batch,"already_imported":true,"rows":[],"posts":false}),context,rows:vec![],balances:BTreeMap::new(),existing:Some(batch)});
        }
        let replacement = self.opening_replacement(c, s, company, input).await?;
        let parsed = crate::opening_csv::parse(csv.as_bytes(), ",")
            .map_err(|e| invalid("csv", format!("cannot read CSV: {e:?}")))?;
        if parsed
            .first()
            .map(|r| r.iter().map(String::as_str).collect::<Vec<_>>())
            != Some(HEADER.to_vec())
            || !(2..=10001).contains(&parsed.len())
            || (input["staged"] != true && (parsed.len() > 501 || csv.len() > 131072))
        {
            return Err(invalid(
                "csv",
                format!(
                    "requires header {} and 1 to 500 rows directly, or up to 10000 rows from staged batches",
                    HEADER.join(",")
                ),
            ));
        }
        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        let wanted: Vec<i64> = parsed
            .iter()
            .skip(1)
            .filter_map(|row| row.get(2).and_then(|id| id.trim().parse().ok()))
            .collect();
        let revisions: BTreeMap<i64, Value> = c
            .query("SELECT id,to_jsonb(r) FROM finance_fixed_assets_category_revisions r WHERE tenant_id=$1 AND company_id=$2 AND id=ANY($3)", &[&s.tenant(), &company, &wanted])
            .await?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect();
        let mut seen = BTreeSet::new();
        let mut rows = vec![];
        let mut balances = BTreeMap::new();
        let mut roles = BTreeMap::new();
        let mut classes = BTreeMap::<(i64, i64, i64), (Money, Money)>::new();
        for (index, row) in parsed.into_iter().enumerate().skip(1) {
            let fail = |text: &str| invalid("csv", format!("row {}: {text}", index + 1));
            if row.len() != HEADER.len() {
                return Err(fail("has the wrong number of columns"));
            }
            let row: Vec<_> = row.iter().map(|v| v.trim()).collect();
            if row[0].is_empty() || row[0].chars().count() > 200 || !seen.insert(row[0].to_owned())
            {
                return Err(fail(
                    "source_id must be unique and contain 1 to 200 characters",
                ));
            }
            if row[1].is_empty() || row[1].chars().count() > 500 {
                return Err(fail("description must contain 1 to 500 characters"));
            }
            let revision: i64 = row[2].parse().map_err(|_| {
                fail(
                    "category_revision_id must identify a category revision in the current company",
                )
            })?;
            let r = revisions
                .get(&revision)
                .ok_or_else(|| fail("category revision is not in the current company"))?;
            let acquisition: Date = row[3]
                .parse()
                .map_err(|_| fail("acquisition_date must be YYYY-MM-DD"))?;
            let service: Date = row[4]
                .parse()
                .map_err(|_| fail("in_service_date must be YYYY-MM-DD"))?;
            if acquisition.year() < 1 || service.year() < 1 {
                return Err(fail(
                    "acquisition and in-service dates must be in year 0001 or later",
                ));
            }
            if acquisition >= context.entry.entry_date
                || service < acquisition
                || service > context.entry.entry_date
            {
                return Err(fail(
                    "acquisition must precede conversion; in-service date must fall between acquisition and conversion",
                ));
            }
            let amount = |n: usize| {
                Money::parse(row[n], book.minor_units).map_err(|_| {
                    fail(
                        "cost, accumulated depreciation and residual value must use book precision",
                    )
                })
            };
            let cost = amount(5)?;
            let accumulated = amount(6)?;
            let residual = amount(8)?;
            if !cost.is_positive()
                || accumulated < Money::ZERO
                || residual < Money::ZERO
                || accumulated + residual > cost
            {
                return Err(fail(
                    "cost must be positive; accumulated depreciation and residual value must be nonnegative and together no greater than cost",
                ));
            }
            let months: i32 = row[7]
                .parse()
                .map_err(|_| fail("remaining_life_months must be 0 to 1200"))?;
            if !(0..=1200).contains(&months) {
                return Err(fail("remaining_life_months must be 0 to 1200"));
            }
            if months == 0 && accumulated + residual != cost {
                return Err(fail(
                    "zero remaining life requires carrying value to equal residual value",
                ));
            }
            use jiff::ToSpan;
            if months > 0 {
                context
                    .entry
                    .entry_date
                    .first_of_month()
                    .checked_add((months - 1).months())
                    .map_err(|_| {
                        fail("the prospective schedule exceeds the supported date range")
                    })?;
            }
            // An opening register freezes an explicit prospective straight-line estimate.
            // No historical schedule is reconstructed or caught up.
            let mut facts = json!({"source_id":row[0],"description":row[1],"category_revision_id":revision,"acquisition_date":acquisition.to_string(),"in_service_date":service.to_string(),"cost":cost,"accumulated_depreciation":accumulated,"remaining_life_months":months,"residual_value":residual,"carrying_value":cost-accumulated,"method":"straight_line","opening_date":context.entry.entry_date.to_string(),"category_method":r["method"]});
            for (field, role, balance) in [
                ("cost_account_id", "cost", cost),
                (
                    "accumulated_account_id",
                    "accumulated",
                    Money::ZERO - accumulated,
                ),
            ] {
                let id = r[field].as_i64().ok_or(Error::Internal)?;
                let a = chart
                    .iter()
                    .find(|a| a.id == id && a.postable())
                    .ok_or_else(|| fail("category accounts must be active and postable"))?;
                if roles.insert(id, role).is_some_and(|old| old != role) {
                    return Err(fail(
                        "one opening account cannot represent both cost and accumulated depreciation",
                    ));
                }
                let total = balances.entry(id).or_insert(Money::ZERO);
                *total = Money::new((*total + balance).amount(), book.minor_units)
                    .map_err(|e| fail(&e.to_string()))?;
                facts[field] = json!(id);
                facts[format!("{field}_code")] = json!(a.code);
            }
            let key = (
                r["category_id"].as_i64().ok_or(Error::Invalid)?,
                facts["cost_account_id"].as_i64().ok_or(Error::Invalid)?,
                facts["accumulated_account_id"]
                    .as_i64()
                    .ok_or(Error::Invalid)?,
            );
            let totals = classes.entry(key).or_insert((Money::ZERO, Money::ZERO));
            totals.0 = Money::new((totals.0 + cost).amount(), book.minor_units)
                .map_err(|e| fail(&e.to_string()))?;
            totals.1 = Money::new((totals.1 + accumulated).amount(), book.minor_units)
                .map_err(|e| fail(&e.to_string()))?;
            rows.push(facts);
        }
        // Class totals explain the source register; only account totals reconcile
        // to the ledger because an account may be shared by several classes.
        let class_totals: Vec<Value> = classes.into_iter().map(|((category, cost_account, accumulated_account), (cost, accumulated))| json!({"category_id":category,"cost_account_id":cost_account,"accumulated_account_id":accumulated_account,"cost":cost,"accumulated_depreciation":accumulated,"carrying_value":cost-accumulated})).collect();
        let mut reconciliation = vec![];
        for (&account, &balance) in &balances {
            let expected = context
                .balances
                .get(&account)
                .copied()
                .unwrap_or(Money::ZERO);
            if expected != balance {
                return Err(invalid(
                    "csv",
                    format!(
                        "account {}: register balance {} does not reconcile to opening balance {} (difference {})",
                        chart
                            .iter()
                            .find(|a| a.id == account)
                            .map_or("", |a| a.code.as_str()),
                        balance.display(book.minor_units),
                        expected.display(book.minor_units),
                        (balance - expected).display(book.minor_units)
                    ),
                ));
            }
            reconciliation.push(json!({"account_id":account,"account_code":chart.iter().find(|a|a.id==account).map(|a|a.code.clone()),"register_balance":balance,"opening_balance":expected,"difference":Money::ZERO}));
        }
        if replacement.is_none() && c.query_opt("SELECT id FROM finance_fixed_assets_assets WHERE tenant_id=$1 AND company_id=$2 AND opening_date IS NULL LIMIT 1",&[&s.tenant(),&company]).await?.is_some() {return Err(invalid("csv","import the opening register before registering ordinary acquisitions"));}
        if let Some(old) = &replacement {
            let old_id = old["id"].as_i64().ok_or(Error::Invalid)?;
            let prior=c.query("SELECT source_facts FROM finance_fixed_assets_opening_items WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&old_id]).await?;
            for row in prior {
                let facts: Value = row.get(0);
                for field in ["cost_account_id", "accumulated_account_id"] {
                    if !facts[field]
                        .as_i64()
                        .is_some_and(|id| balances.contains_key(&id))
                    {
                        return Err(invalid(
                            "csv",
                            "a correction must retain every previously recognized opening account and its balance",
                        ));
                    }
                }
            }
        }
        let approval = self
            .ledger
            .preview_approval_on(
                c,
                s,
                company,
                &self
                    .opening_approval(c, s, company, &context, input)
                    .await?,
            )
            .await?;
        Ok(Prepared {
            review: json!({"state":"previewed","replaces_batch_id":input["replaces_batch_id"],"reason":input["reason"],"staged":input["staged"]==true,"conversion_date":context.entry.entry_date.to_string(),"entry_id":context.entry.id,"entry_number":context.entry.entry_number,"source_reference":reference,"currency_code":book.currency_code,"minor_units":book.minor_units,"rows":rows,"class_totals":class_totals,"month_convention":CONVENTION,"reconciliation":reconciliation,"posts":false,"approval":approval,"already_imported":false,"depreciation":"Prospective straight-line depreciation of source carrying value less residual value over the remaining useful life. A full first month is recognized at month-end, including a mid-month cut-off."}),
            context,
            rows,
            balances,
            existing: None,
        })
    }
    pub async fn import_opening_register(
        &self,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                ACQUIRE,
                "finance.fixed-assets.opening.import",
            )
            .await?;
        let result = self.import_opening_on(&mut tx, s, company, input).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(super) async fn import_opening_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        let resolved = self.resolve_opening(tx.sql(), s, company, input).await?;
        let input = &resolved;
        let p = self.prepare_opening(tx.sql(), s, company, input).await?;
        if let Some(mut batch) = p.existing {
            batch["state"] = json!("reconciled");
            return Ok(batch);
        }
        self.ledger
            .check_approval(
                tx,
                s,
                company,
                &self
                    .opening_approval(tx.sql(), s, company, &p.context, input)
                    .await?,
            )
            .await?;
        let review = p.review.clone();
        let tenant = s.tenant();
        let actor = s.user();
        let imp = s.impersonator().map(|p| p.user());
        let source = input["source_reference"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let csv = input["csv"].clone();
        let entry = p.context.entry;
        let rows = p.rows;
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let (batch,change)=store::insert(sql,store::Table::OpeningBatch,json!({"tenant_id":tenant,"company_id":company,"source_reference":source,"source_csv":csv,"currency_code":entry.currency_code,"conversion_date":entry.entry_date.to_string(),"entry_id":entry.id,"entry_number":entry.entry_number,"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp})).await?;
            let mut changes=vec![change];
            for facts in rows {
                let (asset,change)=store::insert(sql,store::Table::Asset,json!({"tenant_id":tenant,"company_id":company,"asset_number":1,"category_revision_id":facts["category_revision_id"],"description":facts["description"],"acquisition_date":facts["acquisition_date"],"in_service_date":facts["in_service_date"],"cost":facts["cost"],"residual_value":facts["residual_value"],"method":"straight_line","life_months":facts["remaining_life_months"],"entry_id":entry.id,"entry_number":entry.entry_number,"opening_date":facts["opening_date"],"opening_accumulated":facts["accumulated_depreciation"],"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp})).await?;
                changes.push(change);
                changes.push(store::insert(sql,store::Table::OpeningItem,json!({"tenant_id":tenant,"company_id":company,"batch_id":batch["id"],"source_id":facts["source_id"],"asset_id":asset["id"],"source_facts":facts})).await?.1);
            }
            Ok((batch,changes))
        })).await?;
        for (account, balance) in p.balances {
            if balance.is_zero() && !p.context.balances.contains_key(&account) {
                continue;
            }
            self.ledger
                .replace_opening_recognition_on(
                    tx,
                    s,
                    company,
                    &EntryOwner {
                        module: MODULE.into(),
                        record: format!("opening:{}", batch["id"]),
                    },
                    input["replaces_batch_id"]
                        .as_i64()
                        .map(|id| format!("opening:{id}"))
                        .as_deref(),
                    account,
                    balance,
                )
                .await?;
        }
        self.record_opening_decision(tx, s, company, &batch, input, &review)
            .await?;
        let mut batch = batch;
        batch["state"] = json!("reconciled");
        Ok(batch)
    }
}
