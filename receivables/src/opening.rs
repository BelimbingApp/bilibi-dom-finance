//! Conversion residuals use Receivables' settlement records without issuing a
//! new supply or posting the source control balance again.
use crate::*;
use bilibi_agent_api::OperationClient;
use bilibi_database::Change;
use bilibi_finance_ledger::{EntryOwner, Money, OpeningContext};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const HEADER: &[&str] = &[
    "source_id",
    "customer_code",
    "document_reference",
    "document_date",
    "due_date",
    "original_total",
    "outstanding",
    "control_account_code",
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
    balance: Money,
    control: i64,
    series: Value,
    existing: Option<Value>,
}

impl Receivables {
    pub(super) async fn opening_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
    ) -> Result<Value> {
        self.authorize_on(c, s, company, VIEW).await?;
        let opening = self.ledger.opening_context_on(c, s, company).await?;
        let book = self.ledger.get_book_on(c, s, company).await?;
        let items: Vec<Value> = c.query("SELECT to_jsonb(i) FROM finance_receivables_opening_items i WHERE tenant_id=$1 AND company_id=$2 ORDER BY id", &[&s.tenant(), &company]).await?.iter().map(|row| row.get(0)).collect();
        Ok(
            json!({"opening":opening,"book":book,"batch":store::find(c,BATCH,s.tenant(),company,None).await?,"header":HEADER,"items":items}),
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
        let existing = store::find(c, BATCH, s.tenant(), company, None).await?;
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
            return Ok(Prepared {
                review: json!({"batch":batch,"already_imported":true,"rows":[],"posts":false}),
                context,
                rows: vec![],
                balance: Money::ZERO,
                control: 0,
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
        let setup = store::find(c, store::SETUPS, s.tenant(), company, None)
            .await?
            .ok_or(ReceivableError::NotConfigured)?;
        let control = setup["receivable_account_id"]
            .as_i64()
            .ok_or(Error::Internal)?;
        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        let account = chart
            .iter()
            .find(|a| a.id == control && a.postable())
            .ok_or_else(|| {
                invalid(
                    "csv",
                    "the receivables control account must be active and postable",
                )
            })?;
        let customers = store::list(c, store::CUSTOMERS, s.tenant(), company).await?;
        let mut seen = BTreeSet::new();
        let mut documents = BTreeSet::new();
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
            if row[2].is_empty() || row[2].chars().count() > 100 {
                return Err(fail("document_reference must contain 1 to 100 characters"));
            }
            let customer = customers
                .iter()
                .find(|v| v["code"] == row[1] && v["active"] == true)
                .ok_or_else(|| {
                    fail("customer_code must identify an active customer in the current company")
                })?;
            let customer_id = customer["id"].as_i64().ok_or(Error::Internal)?;
            if !documents.insert((customer_id, row[2].to_owned())) {
                return Err(fail("the customer document reference is duplicated"));
            }
            let date: jiff::civil::Date = row[3]
                .parse()
                .map_err(|_| fail("document_date must be YYYY-MM-DD"))?;
            let due: jiff::civil::Date = row[4]
                .parse()
                .map_err(|_| fail("due_date must be YYYY-MM-DD"))?;
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
            if row[7] != account.code {
                return Err(fail(
                    "control_account_code must match the configured receivables control",
                ));
            }
            balance = Money::new((balance + residual).amount(), book.minor_units)
                .map_err(|e| fail(&e.to_string()))?;
            rows.push(json!({"source_id":row[0],"customer_id":customer_id,"customer_code":row[1],"customer_name":customer["name"],"customer":customer,"document_reference":row[2],"document_date":date.to_string(),"due_date":due.to_string(),"original_total":original,"outstanding":residual,"settled_before_conversion":original-residual,"account_id":control,"account_code":account.code}));
        }
        let expected = context
            .balances
            .get(&control)
            .copied()
            .unwrap_or(Money::ZERO);
        if balance != expected {
            return Err(invalid(
                "csv",
                format!(
                    "opening invoices total {} does not reconcile to opening control balance {} (difference {})",
                    balance.display(book.minor_units),
                    expected.display(book.minor_units),
                    (balance - expected).display(book.minor_units)
                ),
            ));
        }
        if c.query_opt("SELECT id FROM finance_receivables_documents WHERE tenant_id=$1 AND company_id=$2 AND status='issued' LIMIT 1",&[&s.tenant(),&company]).await?.is_some() {return Err(invalid("csv","import opening invoices before issuing ordinary invoices"));}
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
        let source = input["source_reference"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let csv = input["csv"].clone();
        let entry = p.context.entry;
        let series = p.series;
        let rows = p.rows;
        let control = p.control;
        let batch=tx.mutate(move |sql|Box::pin(async move {
            let mut changes=vec![];
            let batch=store::save(sql,BATCH,tenant,company,None,json!({"source_reference":source,"source_csv":csv,"currency_code":entry.currency_code,"conversion_date":entry.entry_date.to_string(),"entry_id":entry.id,"entry_number":entry.entry_number,"series_id":series["id"],"recorded_by_user_id":actor,"recorded_by_impersonator_id":imp}),&mut changes).await?;
            for facts in rows {
                let customer=&facts["customer"];
                let total=&facts["outstanding"];
                let old=store::save(sql,store::DOCUMENTS,tenant,company,None,json!({"kind":"invoice","status":"draft","series_id":series["id"],"customer_id":facts["customer_id"],"currency_code":entry.currency_code,"issue_date":facts["document_date"],"due_date":facts["due_date"],"description":format!("Opening invoice {}",facts["document_reference"].as_str().unwrap_or("")),"subtotal":total,"tax_total":"0","total":total}),&mut changes).await?;
                let id=old["id"].as_i64().ok_or(Error::Invalid)?;
                store::save(sql,store::LINES,tenant,company,None,json!({"document_id":id,"line_number":1,"description":"Opening outstanding balance; no new supply","quantity":"1","unit_price":total,"amount":total,"account_id":control,"tax_amount":"0"}),&mut changes).await?;
                let number:i64=sql.query_one("SELECT COALESCE(max(document_number),0)+1 FROM finance_receivables_documents WHERE tenant_id=$1 AND company_id=$2 AND series_id=$3",&[&tenant,&company,&series["id"].as_i64().ok_or(Error::Invalid)?]).await?.get(0);
                let reference=format!("{}{:0width$}",series["prefix"].as_str().unwrap_or(""),number,width=series["number_width"].as_u64().unwrap_or(1) as usize);
                let name=customer["name"].as_str().ok_or(Error::Invalid)?;
                let row=sql.query_one("UPDATE finance_receivables_documents d SET status='issued',document_number=$4,document_reference=$5,receivable_account_id=$6,party_name=$7,issued_by_user_id=$8,issued_by_impersonator_id=$9,ledger_entry_id=$10,ledger_entry_number=$11 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING to_jsonb(d)",&[&tenant,&company,&id,&number,&reference,&control,&name,&actor,&imp,&entry.id,&entry.entry_number]).await?;
                let new:Value=row.get(0);changes.push(Change{kind:store::DOCUMENTS.kind.into(),id,old,new});
                store::save(sql,ITEM,tenant,company,None,json!({"batch_id":batch["id"],"source_id":facts["source_id"],"document_id":id,"source_facts":facts}),&mut changes).await?;
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
                p.balance,
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
    if c.query_opt("SELECT i.id FROM finance_receivables_opening_items i JOIN finance_receivables_opening_batches b ON b.id=i.batch_id AND b.tenant_id=$1 AND b.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=$3 AND b.conversion_date>$4::text::date",&[&s.tenant(),&company,&document,&date.to_string()]).await?.is_some() {return Err(invalid("document_id","cannot be settled before the conversion date"));}
    Ok(())
}
