//! Opening supplier residuals share Payables' settlement records without
//! recording purchases, tax or a second control-account posting.
use crate::*;
use bilibi_database::Change;
use bilibi_finance_ledger::{EntryOwner, OpeningContext};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const HEADER: &[&str] = &[
    "source_id",
    "vendor_key",
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
            json!({"opening":self.ledger.opening_context_on(c,s,company).await?,"book":self.ledger.get_book_on(c,s,company).await?,"batch":c.query_opt("SELECT to_jsonb(b) FROM finance_payables_opening_batches b WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?.map(|r|r.get::<_,Value>(0)),"header":HEADER,"items":items}),
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
        if let Some(row)=c.query_opt("SELECT to_jsonb(b) FROM finance_payables_opening_batches b WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await? {
            let batch:Value=row.get(0);
            if batch["source_reference"]!=reference || batch["source_csv"]!=csv {return Err(invalid("source_reference","opening bills have already been imported; the same source reference and exact CSV are required for a retry"));}
            return Ok(Prepared{review:json!({"batch":batch,"already_imported":true,"rows":[],"posts":false}),context,rows:vec![],balance:Money::ZERO,control:0,existing:Some(batch)});
        }
        let parsed = crate::opening_csv::parse(csv.as_bytes(), ",")
            .map_err(|e| invalid("csv", format!("cannot read CSV: {e:?}")))?;
        if parsed
            .first()
            .map(|r| r.iter().map(String::as_str).collect::<Vec<_>>())
            != Some(HEADER.to_vec())
            || !(2..=501).contains(&parsed.len())
        {
            return Err(invalid(
                "csv",
                format!("requires header {} and 1 to 500 rows", HEADER.join(",")),
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
                || row[1].chars().count() > 100
                || row[2].is_empty()
                || row[2].chars().count() > 200
            {
                return Err(fail(
                    "vendor_key and vendor_name are required, at most 100 and 200 characters",
                ));
            }
            if vendors
                .insert(row[1].to_owned(), row[2].to_owned())
                .is_some_and(|name| name != row[2])
            {
                return Err(fail("vendor_key identifies more than one name"));
            }
            if row[3].is_empty()
                || row[3].chars().count() > 100
                || !docs.insert((row[1].to_owned(), row[3].to_owned()))
            {
                return Err(fail(
                    "document_reference must contain 1 to 100 characters and be unique for the vendor",
                ));
            }
            if c.query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND party_key=$3 AND party_document_number=$4 AND status<>'voided'",&[&s.tenant(),&company,&row[1],&row[3]]).await?.is_some() {return Err(fail("the vendor document is already recorded"));}
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
            rows.push(json!({"source_id":row[0],"vendor_key":row[1],"vendor_name":row[2],"document_reference":row[3],"document_date":date.to_string(),"due_date":due.to_string(),"original_total":original,"outstanding":residual,"settled_before_conversion":original-residual,"account_id":control,"account_code":account.code}));
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
        if c.query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND status='posted' LIMIT 1",&[&s.tenant(),&company]).await?.is_some() {return Err(invalid("csv","import opening bills before posting ordinary bills"));}
        let approval = self
            .ledger
            .preview_approval_on(
                c,
                s,
                company,
                &context.approval(MODULE, input["override_reason"].as_str().map(str::to_owned)),
            )
            .await?;
        Ok(Prepared {
            review: json!({"conversion_date":context.entry.entry_date.to_string(),"entry_id":context.entry.id,"entry_number":context.entry.entry_number,"source_reference":reference,"currency_code":book.currency_code,"minor_units":book.minor_units,"rows":rows,"total":balance,"opening_balance":expected,"difference":Money::ZERO,"posts":false,"approval":approval,"already_imported":false}),
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
        let p = self.prepare_opening(tx.sql(), s, company, input).await?;
        if let Some(batch) = p.existing {
            return Ok(batch);
        }
        self.ledger
            .check_approval(
                tx,
                s,
                company,
                &p.context
                    .approval(MODULE, input["override_reason"].as_str().map(str::to_owned)),
            )
            .await?;
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
        let rows = p.rows;
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let (batch,change)=store::insert(sql,store::Table::OpeningBatch,json!({"tenant_id":tenant,"company_id":company,"source_reference":source,"source_csv":csv,"currency_code":entry.currency_code,"conversion_date":entry.entry_date.to_string(),"entry_id":entry.id,"entry_number":entry.entry_number,"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp,"recorded_at":jiff::Timestamp::now().to_string()})).await?;
            let mut changes=vec![change];
            for facts in rows {
                let (old,change)=store::insert(sql,store::Table::Documents,json!({"tenant_id":tenant,"company_id":company,"document_type":"bill","status":"draft","party_key":facts["vendor_key"],"party_name":facts["vendor_name"],"party_document_number":facts["document_reference"],"document_date":facts["document_date"],"due_date":facts["due_date"],"currency_code":entry.currency_code,"description":"Opening outstanding balance; no new purchase","total_net":facts["outstanding"],"total_tax":"0","total":facts["outstanding"],"created_by_user_id":actor,"created_by_impersonator_id":imp})).await?;
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
            .recognize_opening_on(
                tx,
                s,
                company,
                &EntryOwner {
                    module: MODULE.into(),
                    record: format!("opening:{}", batch["id"]),
                },
                control,
                Money::ZERO - p.balance,
            )
            .await?;
        Ok(batch)
    }
}

/// Refuse settlement before Bilibi owns processing, independently of the
/// original historical document date retained for aging.
pub(crate) async fn settlement_date(
    c: &dyn OperationClient,
    s: &Scope,
    company: i64,
    document: i64,
    date: jiff::civil::Date,
) -> Result<()> {
    if c.query_opt("SELECT i.id FROM finance_payables_opening_items i JOIN finance_payables_opening_batches b ON b.id=i.batch_id AND b.tenant_id=$1 AND b.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3 AND b.conversion_date>$4::text::date",&[&s.tenant(),&company,&document,&date.to_string()]).await?.is_some() {return Err(invalid("document_id","cannot be settled before the conversion date"));}
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
