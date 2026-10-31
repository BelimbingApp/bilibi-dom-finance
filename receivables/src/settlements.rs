//! Immutable opening-credit settlement. Every door joins this owner path and
//! rechecks dated capacity while holding the same company lock as the ledger.
use crate::*;
use bilibi_agent_api::OperationClient;
use bilibi_database::Change;
use bilibi_finance_ledger::{
    Approval, EntryOwner, Money, NewEntry, NewLine, PostOptions, Preparer, Reversal,
};
use jiff::civil::Date;
use serde_json::{json, Value};

pub(super) struct PreparedSettlement {
    pub review: Value,
    data: Value,
    entry: Option<NewEntry>,
    options: PostOptions,
    original: Option<Value>,
    approval: Approval,
    existing: Option<Value>,
}
fn id(input: &Value, key: &'static str) -> Result<i64> {
    input[key]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid(key, "must identify a record"))
}
fn owner(number: i64) -> EntryOwner {
    EntryOwner {
        module: MODULE.into(),
        record: format!("credit-settlement:{number}"),
    }
}
fn reversal(date: Date, reference: &str) -> Reversal {
    Reversal {
        entry_date: date,
        description: "Reverse credit settlement".into(),
        reference: Some(reference.into()),
    }
}

/// Lowest dated available capacity, including all existing notes, receipts,
/// payments, write-offs and their restorations, and every credit event.
pub(crate) async fn lowest(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    document: i64,
    since: Date,
) -> Result<Money> {
    let row = c.query_opt("SELECT min(finance_receivables_residual($1,$2,$3,day,9223372036854775807,9223372036854775807)) FROM (SELECT $4::text::date day UNION SELECT effective_date FROM finance_receivables_settlements WHERE tenant_id=$1 AND company_id=$2 AND (credit_document_id=$3 OR target_document_id=$3) AND effective_date>$4::text::date UNION SELECT issue_date FROM finance_receivables_documents WHERE tenant_id=$1 AND company_id=$2 AND credited_document_id=$3 AND status='issued' AND issue_date>$4::text::date
 UNION SELECT p.payment_date FROM finance_receivables_payments p JOIN finance_receivables_allocations a ON a.payment_id=p.id AND a.tenant_id=$1 AND a.company_id=$2 WHERE p.tenant_id=$1 AND p.company_id=$2 AND a.document_id=$3 AND p.payment_date>$4::text::date
 UNION SELECT x.reversal_date FROM finance_receivables_payment_reversals x JOIN finance_receivables_allocations a ON a.payment_id=x.payment_id AND a.tenant_id=$1 AND a.company_id=$2 WHERE x.tenant_id=$1 AND x.company_id=$2 AND a.document_id=$3 AND x.reversal_date>$4::text::date
 UNION SELECT e.effective_date FROM finance_receivables_write_off_events e JOIN finance_receivables_write_offs w ON w.id=e.write_off_id AND w.tenant_id=$1 AND w.company_id=$2 WHERE e.tenant_id=$1 AND e.company_id=$2 AND w.document_id=$3 AND e.effective_date>$4::text::date) dates", &[&tenant,&company,&document,&since.to_string()]).await?.ok_or(Error::Internal)?;
    let amount: rust_decimal::Decimal = row.get(0);
    Ok(Money::parse(
        &amount.to_string(),
        bilibi_finance_ledger::MinorUnits::new(4).ok_or(Error::Internal)?,
    )
    .map_err(|_| Error::Internal)?)
}

pub(crate) async fn pin(
    c: &dyn OperationClient,
    s: &Scope,
    company: i64,
    through: Option<i64>,
    entry: i64,
    batch: i64,
    day: Date,
) -> Result<i64> {
    let maximum:i64=c.query_opt("SELECT COALESCE(max(settlement_number),0) FROM finance_receivables_settlements WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?.ok_or(Error::Internal)?.get(0);
    let pin = through.unwrap_or(maximum);
    if pin < 0 || pin > maximum {
        return Err(invalid(
            "through_settlement_number",
            "is outside the company's settlement stream",
        ));
    }
    let incompatible=c.query_opt("SELECT e.id FROM finance_receivables_settlements e WHERE e.tenant_id=$1 AND e.company_id=$2 AND e.effective_date<=$6::text::date AND (
     (e.entry_number IS NOT NULL AND ((e.settlement_number<=$3)<>(e.entry_number<=$4)))
     OR (e.settlement_number<=$3 AND (EXISTS(SELECT 1 FROM finance_receivables_opening_items i WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id IN (e.credit_document_id,e.target_document_id) AND i.batch_id>$5)
     OR EXISTS(SELECT 1 FROM finance_receivables_settlements o WHERE o.tenant_id=$1 AND o.company_id=$2 AND o.id=e.reverses_settlement_id AND o.settlement_number>$3)))) LIMIT 1",&[&s.tenant(),&company,&pin,&entry,&batch,&day.to_string()]).await?.is_some();
    if incompatible {
        return Err(invalid("through_settlement_number","the entry, opening-batch and settlement boundaries split a settlement from its entry, source item or original event"));
    }
    Ok(pin)
}

impl Receivables {
    pub async fn apply_credit(&self, s: &Scope, company: i64, input: &Value) -> Result<Value> {
        self.settle_credit(s, company, "apply", input).await
    }
    pub async fn refund_credit(&self, s: &Scope, company: i64, input: &Value) -> Result<Value> {
        self.settle_credit(s, company, "refund", input).await
    }
    pub async fn reverse_settlement(
        &self,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.settle_credit(s, company, "reverse", input).await
    }
    async fn settle_credit(
        &self,
        s: &Scope,
        company: i64,
        action: &str,
        input: &Value,
    ) -> Result<Value> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                POST_PAYMENTS,
                "finance.receivables.credit.settle",
            )
            .await?;
        let result = self
            .settle_credit_on(&mut tx, s, company, action, input)
            .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(super) async fn prepare_credit(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        action: &str,
        input: &Value,
    ) -> Result<PreparedSettlement> {
        bilibi_company::authorize_work_target(c, s, company, POST_PAYMENTS)
            .await
            .map_err(crate::refusal)?;
        // Pool-owning writes already entered Ledger::write. Preview adapters
        // retain this same company lock on their handed transaction.
        let row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE",&[&s.tenant(),&company]).await?.ok_or(ReceivableError::CompanyNotFound)?;
        if row.get::<_, String>(0) == "archived" {
            return Err(ReceivableError::CompanyArchived);
        }
        let reference = input["reference"].as_str().unwrap_or("").trim();
        if reference.is_empty() || reference.chars().count() > 100 || reference.contains('\0') {
            return Err(invalid(
                "reference",
                "must contain 1 to 100 characters without null characters",
            ));
        }
        let day: Date = input["effective_date"]
            .as_str()
            .ok_or_else(|| invalid("effective_date", "is required"))?
            .parse()
            .map_err(|_| invalid("effective_date", "must be YYYY-MM-DD"))?;
        if day.year() < 1 {
            return Err(invalid("effective_date", "must be in year 0001 or later"));
        }
        let book = self.ledger.get_book_on(c, s, company).await?;
        let placeholder = Approval {
            source: MODULE.into(),
            action: format!("credit_{action}"),
            subject_type: "credit_settlement".into(),
            subject_id: 0,
            subject: reference.into(),
            prepared_by: vec![],
            override_reason: input["override_reason"].as_str().map(str::to_owned),
        };
        if let Some(row)=c.query_opt("SELECT to_jsonb(e) FROM finance_receivables_settlements e WHERE tenant_id=$1 AND company_id=$2 AND reference=$3",&[&s.tenant(),&company,&reference]).await? {
            let event:Value=row.get(0);
            if event["review"]["input"]!=*input || event["kind"]!=action {return Err(invalid("reference","already identifies different settlement instructions"));}
            return Ok(PreparedSettlement {review:json!({"already_recorded":true,"event":event}),data:Value::Null,entry:None,options:PostOptions::default(),original:None,approval:placeholder,existing:Some(event)});
        }
        let original = if action == "reverse" {
            let number = id(input, "settlement_id")?;
            let event:Value=c.query_opt("SELECT to_jsonb(e) FROM finance_receivables_settlements e WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND kind<>'reverse' AND NOT EXISTS(SELECT 1 FROM finance_receivables_settlements r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.reverses_settlement_id=e.id)",&[&s.tenant(),&company,&number]).await?.ok_or_else(||invalid("settlement_id","was not found or has already been reversed"))?.get(0);
            if day.to_string() < event["effective_date"].as_str().unwrap_or("").to_owned() {
                return Err(invalid(
                    "effective_date",
                    "cannot precede the original settlement",
                ));
            }
            Some(event)
        } else {
            None
        };
        let credit = original
            .as_ref()
            .and_then(|v| v["credit_document_id"].as_i64())
            .unwrap_or(id(input, "credit_document_id").unwrap_or(0));
        let facts:Value=c.query_opt("SELECT to_jsonb(d)||jsonb_build_object('source',i.source_facts,'item_id',i.id,'batch',to_jsonb(b)) FROM finance_receivables_documents d JOIN finance_receivables_opening_items i ON i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=d.id JOIN finance_receivables_opening_batches b ON b.tenant_id=$1 AND b.company_id=$2 AND b.id=i.batch_id WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.id=$3 AND d.kind='opening_credit'",&[&s.tenant(),&company,&credit]).await?.ok_or_else(||invalid("credit_document_id","must identify an opening credit of the current company"))?.get(0);
        crate::opening::settlement_date(c, s, company, credit, day).await?;
        let amount = if let Some(event) = &original {
            Money::parse(
                &event["amount"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| event["amount"].to_string()),
                book.minor_units,
            )
            .map_err(|_| Error::Internal)?
        } else {
            Money::parse(
                input["amount"]
                    .as_str()
                    .ok_or_else(|| invalid("amount", "must be a decimal string"))?,
                book.minor_units,
            )
            .map_err(|_| invalid("amount", "must be a positive book-precision amount"))?
        };
        if !amount.is_positive() {
            return Err(invalid("amount", "must be positive"));
        }
        let target = if let Some(event) = &original {
            event["target_document_id"].as_i64()
        } else if action == "apply" {
            Some(id(input, "target_document_id")?)
        } else {
            None
        };
        let mut target_facts = Value::Null;
        let mut available_target = Value::Null;
        let credit_control = facts["receivable_account_id"]
            .as_i64()
            .ok_or(Error::Internal)?;
        let mut controls = vec![credit_control];
        let mut preparers = vec![Preparer {
            user_id: facts["batch"]["recorded_by_user_id"]
                .as_i64()
                .ok_or(Error::Internal)?,
            impersonator_id: facts["batch"]["recorded_by_impersonator_id"].as_i64(),
        }];
        let mut lines = vec![];
        if let Some(target) = target {
            target_facts=c.query_opt("SELECT to_jsonb(d) FROM finance_receivables_documents d WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND kind='invoice' AND status='issued'",&[&s.tenant(),&company,&target]).await?.ok_or_else(||invalid("target_document_id","must identify an issued invoice or posted bill of the current company"))?.get(0);
            if target_facts["customer_id"] != facts["customer_id"]
                || target_facts["currency_code"] != facts["currency_code"]
            {
                return Err(invalid(
                    "target_document_id",
                    "must have the same linked party and currency as the credit",
                ));
            }
            if day.to_string() < target_facts["issue_date"].as_str().unwrap_or("").to_owned() {
                return Err(invalid(
                    "effective_date",
                    "cannot precede the target document's date",
                ));
            }
            crate::opening::settlement_date(c, s, company, target, day).await?;
            let control = target_facts["receivable_account_id"]
                .as_i64()
                .ok_or(Error::Internal)?;
            controls.push(control);
            preparers.push(Preparer {
                user_id: target_facts["issued_by_user_id"]
                    .as_i64()
                    .ok_or(Error::Internal)?,
                impersonator_id: target_facts["issued_by_impersonator_id"].as_i64(),
            });
            if original.is_none() && lowest(c, s.tenant(), company, target, day).await? < amount {
                return Err(invalid(
                    "amount",
                    "exceeds the target's available balance at this date or a later recorded date",
                ));
            }
            if control != credit_control {
                lines = vec![
                    NewLine::debit(credit_control, amount),
                    NewLine::credit(control, amount),
                ];
            }
        }
        let cash = if let Some(event) = &original {
            event["cash_account_id"].as_i64()
        } else if action == "refund" {
            Some(id(input, "cash_account_id")?)
        } else {
            None
        };
        let chart = self.ledger.list_accounts_on(c, s, company).await?;
        if let Some(cash) = cash {
            if original.is_none()
                && !chart.iter().any(|a| {
                    a.id == cash
                        && a.postable()
                        && a.account_type == bilibi_finance_ledger::AccountType::Asset
                        && !controls.contains(&a.id)
                })
            {
                return Err(invalid(
                    "cash_account_id",
                    "must be an active posting asset account other than the settlement's controls",
                ));
            }
            lines = vec![
                NewLine::debit(credit_control, amount),
                NewLine::credit(cash, amount),
            ];
        }
        let available = lowest(c, s.tenant(), company, credit, day).await?;
        if original.is_none() && available < amount {
            return Err(invalid(
                "amount",
                "exceeds the credit's available balance at this date or a later recorded date",
            ));
        }
        let approval = Approval {
            subject_id: credit,
            prepared_by: preparers,
            ..placeholder
        };
        let approval_facts = self
            .ledger
            .preview_approval_on(c, s, company, &approval)
            .await?;
        let next:i64=c.query_opt("SELECT COALESCE(max(settlement_number),0)+1 FROM finance_receivables_settlements WHERE tenant_id=$1 AND company_id=$2",&[&s.tenant(),&company]).await?.ok_or(Error::Internal)?.get(0);
        let options = PostOptions {
            settlement_account_ids: controls.clone(),
            owner: Some(owner(next)),
        };
        let entry = if lines.is_empty() {
            None
        } else {
            Some(NewEntry {
                entry_date: day,
                description: format!("Opening credit {action}"),
                reference: Some(reference.into()),
                lines,
            })
        };
        let posting = if let Some(event) = &original {
            if let Some(entry) = event["entry_id"].as_i64() {
                self.ledger
                    .preview_reverse_owned_entry_on(
                        c,
                        s,
                        company,
                        entry,
                        &owner(event["settlement_number"].as_i64().ok_or(Error::Internal)?),
                        &reversal(day, reference),
                    )
                    .await?
            } else {
                Value::Null
            }
        } else if let Some(entry) = &entry {
            self.ledger
                .preview_post_entry_on(c, s, company, entry, &options)
                .await?
        } else {
            Value::Null
        };
        let data = json!({"settlement_number":next,"kind":action,"credit_item_id":facts["item_id"],"credit_document_id":credit,"target_document_id":target,"amount":amount,"effective_date":day.to_string(),"cash_account_id":cash,"reference":reference,"reverses_settlement_id":original.as_ref().map(|e|e["id"].clone())});
        let review = json!({"input":input,"action":action,"credit":facts,"target":target_facts,"available_credit":available,"available_target":available_target,"amount":amount,"effective_date":day.to_string(),"controls":chart.into_iter().filter(|a|controls.contains(&a.id)||Some(a.id)==cash).collect::<Vec<_>>(),"currency_code":book.currency_code,"minor_units":book.minor_units,"posts":!posting.is_null(),"posting":posting,"approval":approval_facts,"original":original,"settlement_number":next});
        Ok(PreparedSettlement {
            review,
            data,
            entry,
            options,
            original,
            approval,
            existing: None,
        })
    }
    pub(super) async fn settle_credit_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        action: &str,
        input: &Value,
    ) -> Result<Value> {
        let prepared = self
            .prepare_credit(tx.sql(), s, company, action, input)
            .await?;
        if let Some(event) = prepared.existing {
            return Ok(event);
        }
        self.ledger
            .check_approval(tx, s, company, &prepared.approval)
            .await?;
        let mut data = prepared.data;
        let entry = if let Some(original) = &prepared.original {
            if let Some(entry) = original["entry_id"].as_i64() {
                Some(
                    self.ledger
                        .reverse_owned_entry(
                            tx,
                            s,
                            company,
                            entry,
                            &owner(
                                original["settlement_number"]
                                    .as_i64()
                                    .ok_or(Error::Internal)?,
                            ),
                            &reversal(
                                data["effective_date"]
                                    .as_str()
                                    .ok_or(Error::Internal)?
                                    .parse()
                                    .map_err(|_| Error::Internal)?,
                                data["reference"].as_str().ok_or(Error::Internal)?,
                            ),
                        )
                        .await?,
                )
            } else {
                None
            }
        } else if let Some(entry) = &prepared.entry {
            Some(
                self.ledger
                    .post_entry_in(tx, s, company, entry, &prepared.options)
                    .await?,
            )
        } else {
            None
        };
        data["entry_id"] = json!(entry.as_ref().map(|e| e.id));
        data["entry_number"] = json!(entry.as_ref().map(|e| e.entry_number));
        data["review"] = prepared.review;
        data["actor_user_id"] = json!(s.user());
        data["actor_impersonator_id"] = json!(s.impersonator().map(|i| i.user()));
        data["audit_trace_id"] = json!(tx.context().trace_id);
        data["audit_action"] = json!(tx.context().action);
        let tenant = s.tenant();
        tx.mutate(move|sql|Box::pin(async move {
            let row=sql.query_one("INSERT INTO finance_receivables_settlements(tenant_id,company_id,settlement_number,kind,credit_item_id,credit_document_id,target_document_id,amount,effective_date,cash_account_id,reference,reverses_settlement_id,entry_id,entry_number,review,actor_user_id,actor_impersonator_id,audit_trace_id,audit_action) SELECT $1,$2,x.settlement_number,x.kind,x.credit_item_id,x.credit_document_id,x.target_document_id,x.amount,x.effective_date,x.cash_account_id,x.reference,x.reverses_settlement_id,x.entry_id,x.entry_number,x.review,x.actor_user_id,x.actor_impersonator_id,x.audit_trace_id,x.audit_action FROM jsonb_populate_record(NULL::finance_receivables_settlements,$3) x RETURNING to_jsonb(finance_receivables_settlements)",&[&tenant,&company,&data]).await?;
            let new:Value=row.get(0);let id=new["id"].as_i64().ok_or(Error::Internal)?;
            Ok((new.clone(),vec![Change {kind:"finance.receivables.settlement".into(),id,old:Value::Null,new}]))
        })).await.map_err(Into::into)
    }
}

impl Receivables {
    pub(super) async fn credit_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        bilibi_company::authorize_work_target(c, s, company, VIEW)
            .await
            .map_err(crate::refusal)?;
        let credit = id(input, "credit_document_id")?;
        let source:Value=c.query_opt("SELECT to_jsonb(d) FROM finance_receivables_documents d WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.id=$3 AND d.kind='opening_credit'",&[&s.tenant(),&company,&credit]).await?.ok_or_else(||invalid("credit_document_id","must identify an opening credit"))?.get(0);
        let search = input["search"].as_str().unwrap_or("");
        let after = input["after_id"].as_i64().unwrap_or(0);
        let rows=c.query("SELECT to_jsonb(d)||jsonb_build_object('available',finance_receivables_residual($1,$2,d.id,'infinity'::date,9223372036854775807,9223372036854775807)) FROM finance_receivables_documents d WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.customer_id::text=$3 AND d.currency_code=$4 AND d.kind='invoice' AND d.status='issued' AND d.id>$5 AND (strpos(lower(COALESCE(d.document_reference,'')),lower($6))>0) AND NOT EXISTS(SELECT 1 FROM finance_receivables_opening_items i JOIN finance_receivables_opening_decisions x ON x.replaces_batch_id=i.batch_id AND x.tenant_id=$1 AND x.company_id=$2 WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.document_id=d.id) AND finance_receivables_residual($1,$2,d.id,'infinity'::date,9223372036854775807,9223372036854775807)>0 ORDER BY d.id LIMIT 101",&[&s.tenant(),&company,&source["customer_id"].as_str().map(str::to_owned).unwrap_or_else(||source["customer_id"].to_string()),&source["currency_code"].as_str().ok_or(Error::Internal)?,&after,&search]).await?;
        let more = rows.len() > 100;
        let targets: Vec<Value> = rows.into_iter().take(100).map(|r| r.get(0)).collect();
        let next = more
            .then(|| targets.last().map(|r| r["id"].clone()))
            .flatten();
        let control = source["receivable_account_id"]
            .as_i64()
            .ok_or(Error::Internal)?;
        let accounts = self
            .ledger
            .list_accounts_on(c, s, company)
            .await?
            .into_iter()
            .filter(|a| {
                a.postable()
                    && a.account_type == bilibi_finance_ledger::AccountType::Asset
                    && a.id != control
            })
            .collect::<Vec<_>>();
        let book = self.ledger.get_book_on(c, s, company).await?;
        Ok(json!({"targets":targets,"next_after_id":next,"cash_accounts":accounts,"book":book}))
    }
}
