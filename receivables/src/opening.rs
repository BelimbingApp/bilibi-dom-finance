//! Conversion residuals use Receivables' settlement records without issuing a
//! new supply or posting the source control balance again.
use crate::*;
use bilibi_agent_api::OperationClient;
use bilibi_database::Change;
use bilibi_finance_ledger::{AccountType, EntryOwner, Money, OpeningContext, PartyLinks};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const HEADER: &[&str] = &[
    "source_id",
    "customer_code",
    "document_reference",
    "document_date",
    "due_date",
    "original_total",
    "outstanding",
    "control_account_code",
    "party_kind",
];
const BATCH: store::Table = store::Table {
    name: "finance_receivables_opening_batches",
    kind: "finance.receivables.opening_batch",
    columns: &[
        "source_reference",
        "source_csv",
        "currency_code",
        "conversion_date",
        "entry_id",
        "entry_number",
        "series_id",
        "recorded_by_user_id",
        "recorded_by_impersonator_id",
    ],
    timestamps: false,
};
const ITEM: store::Table = store::Table {
    name: "finance_receivables_opening_items",
    kind: "finance.receivables.opening_item",
    columns: &["batch_id", "source_id", "document_id", "source_facts"],
    timestamps: false,
};

pub(super) struct Prepared {
    pub review: Value,
    context: OpeningContext,
    rows: Vec<Value>,
    balances: BTreeMap<i64, Money>,
    replaced_accounts: BTreeSet<i64>,
    series: Value,
    existing: Option<Value>,
}

impl Receivables {
    pub(super) async fn opening_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.authorize_on(c, s, company, VIEW).await?;
        let opening = self.ledger.opening_context_on(c, s, company).await?;
        let book = self.ledger.get_book_on(c, s, company).await?;
        let after = input["items_after"].as_i64().unwrap_or(0);
        let limit = input["limit"].as_i64().unwrap_or(200).clamp(1, 500);
        let items: Vec<Value> = c.query("SELECT to_jsonb(i) FROM finance_receivables_opening_items i WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4", &[&s.tenant(), &company, &after, &(limit + 1)]).await?.iter().map(|row| row.get(0)).collect();
        let more = items.len() as i64 > limit;
        let items: Vec<Value> = items.into_iter().take(limit as usize).collect();
        let next = if more {
            items.last().map(|i| i["id"].clone())
        } else {
            None
        };
        let total: i64 = c.query_opt("SELECT count(*) FROM finance_receivables_opening_items WHERE tenant_id=$1 AND company_id=$2", &[&s.tenant(), &company]).await?.ok_or(Error::Internal)?.get(0);
        Ok(
            json!({"opening":opening,"book":book,"batch":c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_receivables_opening_batches b WHERE tenant_id=$1 AND company_id=$2 ORDER BY id DESC LIMIT 1",&[&s.tenant(),&company]).await?.map(|r|r.get::<_,Value>(0)),"stages":c.query("SELECT to_jsonb(s)-'payload'||jsonb_build_object('part_count',payload->'part_count','csv_bytes',octet_length(payload->>'csv')) FROM finance_receivables_opening_stages s WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"decisions":c.query("SELECT to_jsonb(d) FROM finance_receivables_opening_decisions d WHERE tenant_id=$1 AND company_id=$2 ORDER BY id",&[&s.tenant(),&company]).await?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>(),"header":HEADER,"items":items,"item_count":total,"items_next":next}),
        )
    }

    pub(super) async fn prepare_opening(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Prepared> {
        self.join(c, s, company, ISSUE).await?;
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
        let series_id = input["series_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or_else(|| invalid("series_id", "select an invoice series"))?;
        let series = store::find(c, store::SERIES, s.tenant(), company, Some(series_id))
            .await?
            .ok_or(ReceivableError::SeriesNotFound)?;
        let existing = c.query_opt("SELECT to_jsonb(b) || jsonb_build_object('state','reconciled') FROM finance_receivables_opening_batches b WHERE tenant_id=$1 AND company_id=$2 AND source_reference=$3",&[&s.tenant(),&company,&reference]).await?.map(|r|r.get::<_,Value>(0));
        if let Some(batch) = existing {
            if batch["source_reference"] != reference
                || batch["source_csv"] != csv
                || batch["series_id"] != series_id
            {
                return Err(invalid(
                    "source_reference",
                    "opening invoices have already been imported; the same source reference and exact CSV are required for a retry",
                ));
            }
            let retained=c.query_opt("SELECT review FROM finance_receivables_opening_decisions WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&batch["id"].as_i64().ok_or(Error::Invalid)?]).await?;
            if let Some(row) = retained {
                let review: Value = row.get(0);
                if review["replaces_batch_id"] != input["replaces_batch_id"]
                    || review["reason"] != input["reason"]
                {
                    return Err(invalid(
                        "source_reference",
                        "already identifies different correction instructions",
                    ));
                }
            }
            return Ok(Prepared {
                review: json!({"state":"reconciled","batch":batch,"already_imported":true,"rows":[],"posts":false}),
                context,
                rows: vec![],
                balances: BTreeMap::new(),
                replaced_accounts: BTreeSet::new(),
                series,
                existing: Some(batch),
            });
        }
        if series["kind"] != "invoice" || series["active"] != true {
            return Err(invalid(
                "series_id",
                "must be an active invoice series; use a dedicated opening series",
            ));
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
                format!("requires header {} and 1 to 500 rows directly, or up to 10000 rows from staged batches", HEADER.join(",")),
            ));
        }
        let setup = store::find(c, store::SETUPS, s.tenant(), company, None)
            .await?
            .ok_or(ReceivableError::NotConfigured)?;
        let control = setup["receivable_account_id"]
            .as_i64()
            .ok_or(Error::Internal)?;
        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        if !chart.iter().any(|a| a.id == control && a.postable()) {
            return Err(invalid(
                "csv",
                "the receivables control account must be active and postable",
            ));
        }
        let requested: std::collections::HashSet<String> = parsed
            .iter()
            .skip(1)
            .filter_map(|row| row.get(1).map(|v| v.trim().to_owned()))
            .collect();
        let identities = PartyLinks::new(&self.parties)
            .by_codes(c, s, &requested)
            .await?;
        let kinds: Vec<&str> = identities.iter().map(|i| i.record_kind()).collect();
        let ids: Vec<String> = identities
            .iter()
            .map(|i| i.record_id().to_string())
            .collect();
        let by_party: std::collections::HashMap<_, _> = kinds
            .iter()
            .zip(&ids)
            .zip(&identities)
            .map(|((kind, id), identity)| (((*kind).to_owned(), id.clone()), identity))
            .collect();
        let mut customers = std::collections::HashMap::new();
        for row in c
            .query(
                "SELECT id, record_kind, record_id FROM finance_receivables_customers WHERE tenant_id=$1 AND company_id=$2 AND active AND (record_kind, record_id) IN (SELECT * FROM unnest($3::text[], $4::text[]))",
                &[&s.tenant(), &company, &kinds, &ids],
            )
            .await?
        {
            let (id, kind, record): (i64, String, String) = (row.get(0), row.get(1), row.get(2));
            if let Some(party) = by_party.get(&(kind.clone(), record)) {
                customers.insert(
                    (kind, party.code.clone()),
                    json!({"id":id,"code":party.code,"name":party.name,"record_kind":party.record_kind(),"record_id":party.record_id()}),
                );
            }
        }
        let mut seen = BTreeSet::new();
        let mut documents = BTreeSet::new();
        let mut rows = vec![];
        let mut balances = BTreeMap::<i64, Money>::new();
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
            if row[2].is_empty() || row[2].chars().count() > 100 {
                return Err(fail("document_reference must contain 1 to 100 characters"));
            }
            if !["company", "person"].contains(&row[8]) {
                return Err(fail("party_kind must be company or person"));
            }
            if row[1].is_empty() {
                return Err(fail("customer_code is required"));
            }
            let customer = customers.get(&(row[8].to_owned(),row[1].to_owned())).ok_or_else(|| {
                fail("customer_code and party_kind must identify an active Core-linked customer in the current company")
            })?;
            let customer_id = customer["id"].as_i64().ok_or(Error::Internal)?;
            let customer_name = customer["name"].as_str().ok_or(Error::Internal)?;
            if !documents.insert((customer_id, row[2].to_owned())) {
                return Err(fail("the customer document reference is duplicated"));
            }
            let date: jiff::civil::Date = row[3]
                .parse()
                .map_err(|_| fail("document_date must be YYYY-MM-DD"))?;
            let due: jiff::civil::Date = row[4]
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
            let original = Money::parse(row[5], book.minor_units)
                .map_err(|_| fail("original_total must be a book-precision amount"))?;
            let residual = Money::parse(row[6], book.minor_units)
                .map_err(|_| fail("outstanding must be a book-precision amount"))?;
            if !original.is_positive() || !residual.is_positive() || residual > original {
                return Err(fail(
                    "outstanding must be positive and no greater than original_total; opening credits are not supported",
                ));
            }
            let account = chart
                .iter()
                .find(|a| a.code == row[7] && a.account_type == AccountType::Asset && a.postable())
                .ok_or_else(|| {
                    fail("control_account_code must identify an active, postable asset account")
                })?;
            let balance = balances.entry(account.id).or_insert(Money::ZERO);
            *balance = Money::new((*balance + residual).amount(), book.minor_units)
                .map_err(|e| fail(&e.to_string()))?;
            rows.push(json!({"source_id":row[0],"customer_id":customer_id,"customer_code":row[1],"party_kind":row[8],"customer_name":customer_name,"customer":customer,"document_reference":row[2],"document_date":date.to_string(),"due_date":due.to_string(),"original_total":original,"outstanding":residual,"settled_before_conversion":original-residual,"account_id":account.id,"account_code":account.code}));
        }
        if context
            .balances
            .get(&control)
            .is_some_and(|b| *b != Money::ZERO)
            && !balances.contains_key(&control)
        {
            return Err(invalid(
                "csv",
                "the register must include the configured receivables control account",
            ));
        }
        let mut reconciliation = Vec::new();
        for (&id, &balance) in &balances {
            let account = chart.iter().find(|a| a.id == id).ok_or(Error::Internal)?;
            let expected = context.balances.get(&id).copied().unwrap_or(Money::ZERO);
            if balance != expected {
                return Err(invalid("csv", format!("opening invoices on {} total {} but the approved opening balance is {} (difference {})", account.code, balance.display(book.minor_units), expected.display(book.minor_units), (balance - expected).display(book.minor_units))));
            }
            reconciliation.push(json!({"account_id":id,"account_code":account.code,"register_balance":balance,"opening_balance":expected,"difference":Money::ZERO}));
        }
        let account_ids: Vec<i64> = balances.keys().copied().collect();
        if c.query_opt("SELECT id FROM finance_receivables_payments WHERE tenant_id=$1 AND company_id=$2 AND deposit_account_id=ANY($3::bigint[]) LIMIT 1", &[&s.tenant(), &company, &account_ids]).await?.is_some() {
            return Err(invalid("csv", "an account used for receipt deposits cannot be a receivables control"));
        }
        let source_ids: Vec<&str> = rows
            .iter()
            .filter_map(|r| r["source_id"].as_str())
            .collect();
        if c.query_opt("SELECT i.id FROM finance_receivables_opening_items i WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.source_id=ANY($3::text[]) AND i.batch_id<>COALESCE($4::bigint,0::bigint) AND NOT EXISTS(SELECT 1 FROM finance_receivables_opening_decisions d WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.replaces_batch_id=i.batch_id) LIMIT 1", &[&s.tenant(), &company, &source_ids, &input["replaces_batch_id"].as_i64()]).await?.is_some() {
            return Err(invalid("csv", "a source ID is already used by another opening register"));
        }
        if replacement.is_none() && c.query_opt("SELECT id FROM finance_receivables_documents WHERE tenant_id=$1 AND company_id=$2 AND status='issued' AND NOT EXISTS(SELECT 1 FROM finance_receivables_opening_items i WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=finance_receivables_documents.id) LIMIT 1",&[&s.tenant(),&company]).await?.is_some() {return Err(invalid("csv","import opening invoices before issuing ordinary invoices"));}
        let mut replaced_accounts = BTreeSet::new();
        if let Some(old) = &replacement {
            let old_id = old["id"].as_i64().ok_or(Error::Invalid)?;
            let accounts=c.query("SELECT DISTINCT (source_facts->>'account_id')::bigint FROM finance_receivables_opening_items WHERE tenant_id=$1 AND company_id=$2 AND batch_id=$3",&[&s.tenant(),&company,&old_id]).await?;
            for row in accounts {
                let id: i64 = row.get(0);
                if !balances.contains_key(&id) {
                    return Err(invalid("csv","a correction must retain every previously recognized opening account and its balance"));
                }
                replaced_accounts.insert(id);
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
            review: json!({"state":"previewed","replaces_batch_id":input["replaces_batch_id"],"reason":input["reason"],"staged":input["staged"]==true,"conversion_date":context.entry.entry_date.to_string(),"entry_id":context.entry.id,"entry_number":context.entry.entry_number,"source_reference":reference,"currency_code":book.currency_code,"minor_units":book.minor_units,"rows":rows,"reconciliation":reconciliation,"posts":false,"approval":approval,"already_imported":false}),
            context,
            rows,
            balances,
            replaced_accounts,
            series,
            existing: None,
        })
    }

    /// Import a complete, reconciled opening receivables register. No entry or
    /// tax lines are created. Identical retries return the retained batch.
    pub async fn import_opening_invoices(
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
                ISSUE,
                "finance.receivables.opening.import",
            )
            .await?;
        let record = self.import_opening_on(&mut tx, s, company, input).await?;
        tx.commit().await?;
        Ok(record)
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
        let series = p.series;
        let rows = p.rows;
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let mut changes=vec![];
            let batch=store::save(sql,BATCH,tenant,company,None,json!({"source_reference":source,"source_csv":csv,"currency_code":entry.currency_code,"conversion_date":entry.entry_date.to_string(),"entry_id":entry.id,"entry_number":entry.entry_number,"series_id":series["id"],"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp}),&mut changes).await?;
            for facts in rows {
                let control = facts["account_id"].as_i64().ok_or(Error::Invalid)?;
                let total=&facts["outstanding"];
                let old=store::save(sql,store::DOCUMENTS,tenant,company,None,json!({"kind":"invoice","status":"draft","series_id":series["id"],"customer_id":facts["customer_id"],"currency_code":entry.currency_code,"issue_date":facts["document_date"],"due_date":facts["due_date"],"description":format!("Opening invoice {}",facts["document_reference"].as_str().unwrap_or("")),"subtotal":total,"tax_total":"0","total":total}),&mut changes).await?;
                let id=old["id"].as_i64().ok_or(Error::Invalid)?;
                store::save(sql,store::LINES,tenant,company,None,json!({"document_id":id,"line_number":1,"description":"Opening outstanding balance; no new supply","quantity":"1","unit_price":total,"amount":total,"account_id":control,"tax_amount":"0"}),&mut changes).await?;
                let number:i64=sql.query_one("SELECT COALESCE(max(document_number),0)+1 FROM finance_receivables_documents WHERE tenant_id=$1 AND company_id=$2 AND series_id=$3",&[&tenant,&company,&series["id"].as_i64().ok_or(Error::Invalid)?]).await?.get(0);
                let reference=format!("{}{:0width$}",series["prefix"].as_str().unwrap_or(""),number,width=series["number_width"].as_u64().unwrap_or(1) as usize);
                let name=facts["customer_name"].as_str().ok_or(Error::Invalid)?;
                let row=sql.query_one("UPDATE finance_receivables_documents d SET status='issued',document_number=$4,document_reference=$5,receivable_account_id=$6,party_name=$7,party_code=$12,issued_by_user_id=$8,issued_by_impersonator_id=$9,ledger_entry_id=$10,ledger_entry_number=$11 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(d)",&[&tenant,&company,&id,&number,&reference,&control,&name,&actor,&imp,&entry.id,&entry.entry_number,&facts["customer_code"].as_str().ok_or(Error::Invalid)?]).await?;
                let new:Value=row.get(0);changes.push(Change{kind:store::DOCUMENTS.kind.into(),id,old,new});
                store::save(sql,ITEM,tenant,company,None,json!({"batch_id":batch["id"],"source_id":facts["source_id"],"document_id":id,"source_facts":facts}),&mut changes).await?;
            }
            Ok((batch,changes))
        })).await?;
        let previous = input["replaces_batch_id"]
            .as_i64()
            .map(|id| format!("opening:{id}"));
        for (control, balance) in p.balances {
            self.ledger
                .replace_opening_recognition_on(
                    tx,
                    s,
                    company,
                    &EntryOwner {
                        module: MODULE.into(),
                        record: format!("opening:{}", batch["id"]),
                    },
                    if p.replaced_accounts.contains(&control) {
                        previous.as_deref()
                    } else {
                        None
                    },
                    control,
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

/// Refuse settlement before Bilibi owns processing, independently of the
/// original historical document date retained for aging.
pub(crate) async fn settlement_date(
    c: &dyn OperationClient,
    s: &Scope,
    company: i64,
    document: i64,
    date: jiff::civil::Date,
) -> Result<()> {
    if c.query_opt("SELECT i.id FROM finance_receivables_opening_items i JOIN finance_receivables_opening_decisions d ON d.replaces_batch_id=i.batch_id AND d.tenant_id=$1 AND d.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3",&[&s.tenant(),&company,&document]).await?.is_some() {return Err(invalid("document_id","belongs to a replaced opening register; use the replacement record"));}
    if c.query_opt("SELECT i.id FROM finance_receivables_opening_items i JOIN finance_receivables_opening_batches b ON b.id=i.batch_id AND b.tenant_id=$1 AND b.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3 AND b.conversion_date>$4::text::date",&[&s.tenant(),&company,&document,&date.to_string()]).await?.is_some() {return Err(invalid("document_id","cannot be settled before the conversion date"));}
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
        return Err(ReceivableError::InvalidOption);
    }
    match through {Some(id)=>Ok(id),None=>Ok(c.query_opt("SELECT COALESCE(max(id),0) FROM finance_receivables_opening_batches WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?.ok_or(Error::Internal)?.get(0))}
}
