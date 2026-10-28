//! Opening supplier residuals share Payables' settlement records without
//! recording purchases, tax or a second control-account posting.
use crate::*;
use bilibi_database::Change;
use bilibi_finance_ledger::{EntryOwner, OpeningContext};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) const HEADER: &[&str] = &[
    "source_id",
    "vendor_code",
    "vendor_name",
    "document_reference",
    "document_date",
    "due_date",
    "original_total",
    "outstanding",
    "control_account_code",
];
pub(super) struct Prepared {
    pub review: Value,
    context: OpeningContext,
    rows: Vec<Value>,
    balance: Money,
    control: i64,
    existing: Option<Value>,
}

// Vendor controls name vendor_id on ordinary bills; an import edits the CSV.
fn vendor_refusal(error: PayablesError, source: &str) -> PayablesError {
    let fields = match error {
        PayablesError::Invalid(fields) | PayablesError::Ledger(LedgerError::Invalid(fields)) => {
            fields
        }
        other => return other,
    };
    PayablesError::Invalid(
        fields
            .into_iter()
            .map(|field| FieldError::new("csv", format!("{source}: vendor_code {}", field.message)))
            .collect(),
    )
}

impl Payables {
    pub(super) async fn opening_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
    ) -> Result<Value> {
        crate::agent_operations::authorize_on(c, s, company, VIEW).await?;
        let items: Vec<Value> = c.query("SELECT to_jsonb(i) FROM finance_payables_opening_items i WHERE tenant_id=$1 AND company_id=$2 ORDER BY id", &[&s.tenant(), &company]).await?.iter().map(|row| row.get(0)).collect();
        Ok(
            json!({"opening":self.ledger.opening_context_on(c,s,company).await?,"book":self.ledger.get_book_on(c,s,company).await?,"batch":c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_payables_opening_batches b WHERE tenant_id=$1 AND company_id=$2 ORDER BY id DESC LIMIT 1",&[&s.tenant(),&company]).await?.map(|r|r.get::<_,Value>(0)),"stages":c.query("SELECT to_jsonb(s) FROM finance_payables_opening_stages s WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"decisions":c.query("SELECT to_jsonb(d) FROM finance_payables_opening_decisions d WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"header":HEADER,"items":items}),
        )
    }
    pub(super) async fn prepare_opening(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Prepared> {
        crate::agent_operations::authorize_on(c, s, company, POST_DOCUMENTS).await?;
        let company_row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE NOWAIT",&[&s.tenant(),&company]).await?.ok_or(PayablesError::CompanyNotFound)?;
        if company_row.get::<_, String>(0) == "archived" {
            return Err(PayablesError::CompanyArchived);
        }
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
        if let Some(row)=c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_payables_opening_batches b WHERE tenant_id=$1 AND company_id=$2 AND source_reference=$3",&[&s.tenant(),&company,&reference]).await? {
            let batch:Value=row.get(0);
            if batch["source_reference"]!=reference || batch["source_csv"]!=csv {return Err(invalid("source_reference","opening bills have already been imported; the same source reference and exact CSV are required for a retry"));}
            let retained=c.query_opt("SELECT review FROM finance_payables_opening_decisions WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&batch["id"].as_i64().ok_or(Error::Invalid)?]).await?;
            if let Some(row)=retained {let review:Value=row.get(0);if review["replaces_batch_id"]!=input["replaces_batch_id"] || review["reason"]!=input["reason"] || review["effective_date"]!=input["effective_date"] {return Err(invalid("source_reference","already identifies different correction instructions"));}}
            return Ok(Prepared{review:json!({"state":"reconciled","batch":batch,"already_imported":true,"rows":[],"posts":false}),context,rows:vec![],balance:Money::ZERO,control:0,existing:Some(batch)});
        }
        let replacement = self.opening_replacement(c, s, company, input).await?;
        let parsed = crate::opening_csv::parse(csv.as_bytes(), ",")
            .map_err(|e| invalid("csv", format!("cannot read CSV: {e:?}")))?;
        if parsed
            .first()
            .map(|r| r.iter().map(String::as_str).collect::<Vec<_>>())
            != Some(HEADER.to_vec())
            || !(2..=100001).contains(&parsed.len())
            || (input["staged"] != true && (parsed.len() > 501 || csv.len() > 131072))
        {
            return Err(invalid(
                "csv",
                format!("requires header {} and 1 to 500 rows directly, or up to 100000 rows from staged batches", HEADER.join(",")),
            ));
        }
        let (_, setup) = store::settings(c, s.tenant(), company)
            .await?
            .ok_or(PayablesError::PayablesNotConfigured)?;
        let control = setup.payable_account_id;
        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        let account = chart
            .iter()
            .find(|a| a.id == control && a.postable())
            .ok_or_else(|| {
                invalid(
                    "csv",
                    "the payables control account must be active and postable",
                )
            })?;
        let mut seen = BTreeSet::new();
        let mut docs = BTreeSet::new();
        let mut vendors = std::collections::BTreeMap::new();
        let mut rows = vec![];
        let mut balance = Money::ZERO;
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
            if row[1].is_empty()
                || row[1].chars().count() > 50
                || row[2].is_empty()
                || row[2].chars().count() > 200
            {
                return Err(fail(
                    "vendor_code and vendor_name are required, at most 50 and 200 characters",
                ));
            }
            if vendors
                .insert(row[1].to_owned(), row[2].to_owned())
                .is_some_and(|name| name != row[2])
            {
                return Err(fail("vendor_code identifies more than one name"));
            }
            if row[3].is_empty()
                || row[3].chars().count() > 100
                || !docs.insert((row[1].to_owned(), row[3].to_owned()))
            {
                return Err(fail(
                    "document_reference must contain 1 to 100 characters and be unique for the vendor",
                ));
            }
            let vendor_id = c.query_opt("SELECT id FROM finance_payables_vendors WHERE tenant_id=$1 AND company_id=$2 AND code=$3", &[&s.tenant(), &company, &row[1]]).await?
                .ok_or_else(|| fail("vendor_code must identify an existing vendor of the current company"))?.get(0);
            let vendor = self.vendor_on(c, s, company, vendor_id).await?;
            Self::billable(&vendor)
                .map_err(|e| vendor_refusal(e, &format!("row {}", index + 1)))?;
            let party_key = vendors::key(vendor_id);
            if c.query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND (party_key=$3 OR party_key IN (SELECT supplier_key FROM finance_payables_vendor_keys WHERE tenant_id=$1 AND company_id=$2 AND vendor_id=$5)) AND party_document_number=$4 AND status<>'voided' AND NOT EXISTS(SELECT 1 FROM finance_payables_opening_items i WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=finance_payables_documents.id AND (i.batch_id=$6 OR EXISTS(SELECT 1 FROM finance_payables_opening_decisions x WHERE x.tenant_id=$1 AND x.company_id=$2 AND x.replaces_batch_id=i.batch_id)))",&[&s.tenant(),&company,&party_key,&row[3],&vendor_id,&input["replaces_batch_id"].as_i64()]).await?.is_some() {return Err(fail("the vendor document is already recorded"));}
            let date: Date = row[4]
                .parse()
                .map_err(|_| fail("document_date must be YYYY-MM-DD"))?;
            let due: Date = row[5]
                .parse()
                .map_err(|_| fail("due_date must be YYYY-MM-DD"))?;
            if date.year() < 1 || due.year() < 1 {
                return Err(fail("document and due dates must be in year 0001 or later"));
            }
            if date >= context.entry.entry_date || due < date {
                return Err(fail(
                    "document_date must precede conversion and due_date must not precede document_date",
                ));
            }
            let original = Money::parse(row[6], book.minor_units)
                .map_err(|_| fail("original_total must be a book-precision amount"))?;
            let residual = Money::parse(row[7], book.minor_units)
                .map_err(|_| fail("outstanding must be a book-precision amount"))?;
            if !original.is_positive() || !residual.is_positive() || residual > original {
                return Err(fail(
                    "outstanding must be positive and no greater than original_total; opening credits are not supported",
                ));
            }
            if row[8] != account.code {
                return Err(fail(
                    "control_account_code must match the configured payables control",
                ));
            }
            balance = Money::new((balance + residual).amount(), book.minor_units)
                .map_err(|e| fail(&e.to_string()))?;
            rows.push(json!({"source_id":row[0],"vendor_code":row[1],"vendor_name":row[2],"vendor_id":vendor_id,"vendor":vendor,"party_key":party_key,"document_reference":row[3],"document_date":date.to_string(),"due_date":due.to_string(),"original_total":original,"outstanding":residual,"settled_before_conversion":original-residual,"account_id":control,"account_code":account.code}));
        }
        let expected = Money::ZERO
            - context
                .balances
                .get(&control)
                .copied()
                .unwrap_or(Money::ZERO);
        if balance != expected {
            return Err(invalid(
                "csv",
                format!(
                    "opening bills total {} does not reconcile to opening control balance {} (difference {})",
                    balance.display(book.minor_units),
                    expected.display(book.minor_units),
                    (balance - expected).display(book.minor_units)
                ),
            ));
        }
        if replacement.is_none() && c.query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND status='posted' AND NOT EXISTS(SELECT 1 FROM finance_payables_opening_items i WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=finance_payables_documents.id) LIMIT 1",&[&s.tenant(),&company]).await?.is_some() {return Err(invalid("csv","import opening bills before posting ordinary bills"));}
        if let Some(old) = &replacement {
            let old_id = old["id"].as_i64().ok_or(Error::Invalid)?;
            let accounts=c.query("SELECT DISTINCT (source_facts->>'account_id')::bigint FROM finance_payables_opening_items WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&old_id]).await?;
            if accounts.iter().any(|r| r.get::<_, i64>(0) != control) {
                return Err(invalid("csv","a correction must retain every previously recognized opening account and its balance"));
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
            review: json!({"state":"previewed","replaces_batch_id":input["replaces_batch_id"],"reason":input["reason"],"effective_date":input["effective_date"],"staged":input["staged"]==true,"conversion_date":context.entry.entry_date.to_string(),"entry_id":context.entry.id,"entry_number":context.entry.entry_number,"source_reference":reference,"currency_code":book.currency_code,"minor_units":book.minor_units,"rows":rows,"total":balance,"opening_balance":expected,"difference":Money::ZERO,"posts":false,"approval":approval,"already_imported":false}),
            context,
            rows,
            balance,
            control,
            existing: None,
        })
    }
    pub async fn import_opening_bills(
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
                POST_DOCUMENTS,
                "finance.payables.opening.import",
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
        join(tx, s, company, POST_DOCUMENTS).await?;
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
        let control = p.control;
        let source = input["source_reference"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let csv = input["csv"].clone();
        let entry = p.context.entry;
        let mut rows = p.rows;
        for facts in &mut rows {
            let core_company = facts["vendor"]["company"]["id"]
                .as_i64()
                .ok_or(Error::Invalid)?;
            facts["party"] = self
                .frozen_vendor_party(tx.sql(), s, core_company)
                .await
                .map_err(|e| {
                    vendor_refusal(
                        e,
                        &format!("source {}", facts["source_id"].as_str().unwrap_or("")),
                    )
                })?;
        }
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let (batch,change)=store::insert(sql,store::Table::OpeningBatch,json!({"tenant_id":tenant,"company_id":company,"source_reference":source,"source_csv":csv,"currency_code":entry.currency_code,"conversion_date":entry.entry_date.to_string(),"entry_id":entry.id,"entry_number":entry.entry_number,"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp,"recorded_at":jiff::Timestamp::now().to_string()})).await?;
            let mut changes=vec![change];
            for facts in rows {
                let (old,change)=store::insert(sql,store::Table::Documents,json!({"tenant_id":tenant,"company_id":company,"opening_batch_id":batch["id"],"document_type":"bill","status":"draft","party_key":facts["party_key"],"party_name":facts["party"]["party_name"],"party_tax_id":facts["party"]["party_tax_identifier"],"party_registration_id":facts["party"]["party_registration_identifier"],"party_document_number":facts["document_reference"],"document_date":facts["document_date"],"due_date":facts["due_date"],"currency_code":entry.currency_code,"description":"Opening outstanding balance; no new purchase","total_net":facts["outstanding"],"total_tax":"0","total":facts["outstanding"],"created_by_user_id":actor,"created_by_impersonator_id":imp})).await?;
                let id=old["id"].as_i64().ok_or(Error::Invalid)?;changes.push(change);
                changes.push(store::insert(sql,store::Table::Lines,json!({"tenant_id":tenant,"company_id":company,"document_id":id,"line_number":1,"description":"Opening outstanding balance; no new purchase","account_id":control,"amount":facts["outstanding"]})).await?.1);
                let approved:Value=sql.query_one("UPDATE finance_payables_documents d SET status='approved',approved_by_user_id=$4,approved_by_impersonator_id=$5 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(d)",&[&tenant,&company,&id,&actor,&imp]).await?.get(0);
                changes.push(Change{kind:"finance.payables.document".into(),id,old,new:approved.clone()});
                let posted:Value=sql.query_one("UPDATE finance_payables_documents d SET status='posted',posted_by_user_id=$4,posted_by_impersonator_id=$5,payable_account_id=$6,entry_id=$7,entry_number=$8 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(d)",&[&tenant,&company,&id,&actor,&imp,&control,&entry.id,&entry.entry_number]).await?.get(0);
                changes.push(Change{kind:"finance.payables.document".into(),id,old:approved,new:posted});
                changes.push(store::insert(sql,store::Table::OpeningItem,json!({"tenant_id":tenant,"company_id":company,"batch_id":batch["id"],"source_id":facts["source_id"],"document_id":id,"source_facts":facts})).await?.1);
            }
            Ok((batch,changes))
        })).await?;
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
                control,
                Money::ZERO - p.balance,
            )
            .await?;
        self.record_opening_decision(tx, s, company, &batch, input, &review)
            .await?;
        let mut batch = batch;
        batch["state"] = json!("reconciled");
        Ok(batch)
    }
}

/// Refuse settlement before Bilibi owns processing, independently of the
/// original historical document date retained for aging.
pub(crate) async fn settlement_date(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    document: i64,
    date: jiff::civil::Date,
    field: &'static str,
) -> Result<()> {
    if c.query_opt("SELECT i.id FROM finance_payables_opening_items i JOIN finance_payables_opening_decisions d ON d.batch_id=i.batch_id AND d.tenant_id=$1 AND d.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3 AND d.replaces_batch_id IS NOT NULL AND d.effective_date>$4::text::date",&[&tenant,&company,&document,&date.to_string()]).await?.is_some() {return Err(invalid(field,"cannot be settled before the register's correction date"));}
    if c.query_opt("SELECT i.id FROM finance_payables_opening_items i JOIN finance_payables_opening_decisions d ON d.replaces_batch_id=i.batch_id AND d.tenant_id=$1 AND d.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3",&[&tenant,&company,&document]).await?.is_some() {return Err(invalid(field,"belongs to a replaced opening register; use the replacement record"));}
    if c.query_opt("SELECT i.id FROM finance_payables_opening_items i JOIN finance_payables_opening_batches b ON b.id=i.batch_id AND b.tenant_id=$1 AND b.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3 AND b.conversion_date>$4::text::date",&[&tenant,&company,&document,&date.to_string()]).await?.is_some() {return Err(invalid(field,"cannot be settled before the conversion date"));}
    Ok(())
}

/// Opening records add no posting, so a reproduced subledger report also keeps
/// this independent immutable-register boundary.
pub(crate) async fn pin(
    c: &dyn bilibi_agent_api::OperationClient,
    s: &Scope,
    company: i64,
    through: Option<i64>,
) -> Result<i64> {
    if through.is_some_and(|id| id < 0) {
        return Err(PayablesError::InvalidOption);
    }
    match through {Some(id)=>Ok(id),None=>Ok(c.query_opt("SELECT COALESCE(max(id),0) FROM finance_payables_opening_batches WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?.ok_or(Error::Internal)?.get(0))}
}
