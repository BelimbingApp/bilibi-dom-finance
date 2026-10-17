//! Bilimbi.Finance.Payables document recording, approval, void and owned post.
use crate::*;
use crate::{
    settings::{account, optional, string},
    store::Table,
};
use bilibi_finance_ledger::{
    AccountType, Approval, EntryOwner, MinorUnits, NewEntry, NewLine, PostOptions, Preparer,
};
use rust_decimal::RoundingStrategy;
use serde_json::json;

fn checked(amount: Money, units: MinorUnits, field: &'static str, positive: bool) -> Result<Money> {
    let amount = Money::new(amount.amount(), units).map_err(|e| invalid(field, e.to_string()))?;
    if amount < Money::ZERO || (positive && amount.is_zero()) {
        return Err(invalid(
            field,
            if positive {
                "must be greater than 0"
            } else {
                "must be greater than or equal to 0"
            },
        ));
    }
    Ok(amount)
}
fn filters(kind: &Option<String>, status: &Option<String>) -> Result<()> {
    if kind
        .as_ref()
        .is_some_and(|k| !matches!(k.as_str(), "bill" | "debit_note"))
        || status.as_ref().is_some_and(|k| {
            !matches!(
                k.as_str(),
                "draft" | "approved" | "held" | "posted" | "voided"
            )
        })
    {
        return Err(PayablesError::InvalidOption);
    }
    Ok(())
}
impl Payables {
    pub async fn create_bill(
        &self,
        s: &Scope,
        company: i64,
        input: &NewDocument,
    ) -> Result<Document> {
        self.create_document(s, company, None, input).await
    }
    pub async fn create_debit_note(
        &self,
        s: &Scope,
        company: i64,
        bill: i64,
        input: &NewDocument,
    ) -> Result<Document> {
        self.create_document(s, company, Some(bill), input).await
    }
    async fn create_document(
        &self,
        s: &Scope,
        company: i64,
        bill: Option<i64>,
        input: &NewDocument,
    ) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                CREATE_DOCUMENTS,
                "finance.payables.create_document",
            )
            .await?;
        let tenant = s.tenant();
        let actor = s.user();
        let imp = s.impersonator().map(|p| p.user());
        let original = if let Some(id) = bill {
            let b = store::document(tx.sql(), tenant, company, id)
                .await?
                .ok_or(PayablesError::DocumentNotFound)?;
            if b.document_type != "bill" || b.status != "posted" {
                return Err(PayablesError::NotPostedBill);
            }
            if input.document_date < b.document_date {
                return Err(invalid(
                    "document_date",
                    "cannot be before the corrected bill's date",
                ));
            }
            Some(b)
        } else {
            None
        };
        let party = original.as_ref().map_or(&input.party, |b| &b.party);
        let key =
            string(&party.key, "party_key", 100, true)?.ok_or(PayablesError::InvalidOption)?;
        let name =
            string(&party.name, "party_name", 200, true)?.ok_or(PayablesError::InvalidOption)?;
        let tax_id = optional(&party.tax_id, "party_tax_id", 50)?;
        let registration = optional(&party.registration_id, "party_registration_id", 50)?;
        let address = optional(&party.address, "party_address", 1000)?;
        let number = string(
            input.party_document_number.as_deref().unwrap_or_default(),
            "party_document_number",
            100,
            bill.is_none(),
        )?;
        let description = optional(&input.description, "description", 500)?;
        let due = input.due_date.unwrap_or(input.document_date);
        if due < input.document_date {
            return Err(invalid("due_date", "cannot be before the document date"));
        }
        if bill.is_none() && tx.sql().query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND party_key=$3 AND party_document_number=$4 AND document_type='bill' AND status<>'voided'",&[&tenant,&company,&key,&number]).await?.is_some() {
            return Err(invalid("party_document_number","is already recorded for this party"));
        }
        let book = self.ledger.get_book_on(tx.sql(), s, company).await?;
        let accounts = self.ledger.list_accounts_on(tx.sql(), s, company).await?;
        let codes = store::taxes(tx.sql(), tenant, company).await?;
        if input.lines.is_empty() {
            return Err(invalid("lines", "needs at least one line"));
        }
        if input.lines.len() > 500 {
            return Err(invalid("lines", "should have at most 500 item(s)"));
        }
        let mut lines = Vec::new();
        let mut taxable: Vec<(i64, Money)> = Vec::new();
        let mut net = Money::ZERO;
        for (i, line) in input.lines.iter().enumerate() {
            let validation = (|| -> Result<(String, Money, Option<String>)> {
                let description = string(&line.description, "description", 500, true)?
                    .ok_or(PayablesError::InvalidOption)?;
                account(
                    &accounts,
                    line.account_id,
                    "account_id",
                    &[AccountType::Asset, AccountType::Expense],
                )?;
                let amount = checked(line.amount, book.minor_units, "amount", true)?;
                let external =
                    optional(&line.external_order_line_id, "external_order_line_id", 100)?;
                if line.order_line_id.is_some_and(|id| id < 1) {
                    return Err(invalid("order_line_id", "must be greater than 0"));
                }
                if let Some(q) = line.quantity {
                    if q <= Decimal::ZERO {
                        return Err(invalid("quantity", "must be greater than 0"));
                    }
                    if q >= Decimal::from(10_i64.pow(16)) {
                        return Err(invalid("quantity", "must be less than 1E+16"));
                    }
                    if q.normalize().scale() > 4 {
                        return Err(invalid("quantity", "has more than 4 decimal places"));
                    }
                }
                match (line.order_line_id, line.quantity) {
                    (None, Some(_)) => {
                        return Err(invalid("order_line_id", "is needed with a quantity"))
                    }
                    (Some(_), None) => {
                        return Err(invalid("quantity", "is needed with an order line"))
                    }
                    // A bill's order lines are checked against Payables' copy of the
                    // order source below; a debit note's against its bill.
                    (Some(_), Some(_)) if bill.is_none() && !self.order_source => {
                        return Err(invalid("order_line_id", "names no installed order source"));
                    }
                    _ => {}
                }
                if let Some(id) = line.tax_code_id {
                    let code = codes.iter().find(|c| c.id == id).ok_or_else(|| {
                        invalid("tax_code_id", "is not a tax code of this company")
                    })?;
                    if !code.active {
                        return Err(invalid("tax_code_id", "is inactive"));
                    }
                }
                Ok((description, amount, external))
            })();
            let (description, amount, external) = validation.map_err(|e| match e {
                PayablesError::Invalid(mut errors) => {
                    for e in &mut errors {
                        e.line = Some(i + 1);
                    }
                    PayablesError::Invalid(errors)
                }
                e => e,
            })?;
            net = net + amount;
            if let Some(id) = line.tax_code_id {
                if let Some((_, sum)) = taxable.iter_mut().find(|(k, _)| *k == id) {
                    *sum = *sum + amount;
                } else {
                    taxable.push((id, amount));
                }
            }
            lines.push(json!({"tenant_id":tenant,"company_id":company,"line_number":i+1,"description":description,"account_id":line.account_id,"amount":amount,"tax_code_id":line.tax_code_id,"external_order_line_id":external,"order_line_id":line.order_line_id,"quantity":line.quantity}));
        }
        checked(net, book.minor_units, "total_net", true)?;
        let named: Vec<(usize, Option<i64>, Option<Decimal>)> = input
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| (i + 1, l.order_line_id, l.quantity))
            .collect();
        if let Some(b) = &original {
            matching::within_billed(tx.sql(), tenant, company, b, &named, None).await?;
        } else {
            let ids: Vec<i64> = named.iter().filter_map(|n| n.1).collect();
            let known = orders::known(tx.sql(), tenant, company, &ids).await?;
            let missing: Vec<FieldError> = named
                .iter()
                .filter(|n| n.1.is_some_and(|id| !known.contains(&id)))
                .map(|n| FieldError {
                    line: Some(n.0),
                    ..FieldError::new(
                        "order_line_id",
                        "is not an order line of this company in the order source",
                    )
                })
                .collect();
            if !missing.is_empty() {
                return Err(PayablesError::Invalid(missing));
            }
        }
        let mut stated = std::collections::BTreeMap::new();
        if let Some(taxes) = &input.tax_lines {
            for tax in taxes {
                if !taxable.iter().any(|(id, _)| *id == tax.tax_code_id) {
                    return Err(invalid("tax_lines", "names a tax code no line uses"));
                }
                let amount = checked(tax.tax_amount, book.minor_units, "tax_lines", false)?;
                if stated.insert(tax.tax_code_id, amount).is_some() {
                    return Err(invalid("tax_lines", "names a tax code twice"));
                }
            }
        }
        let mut taxes = Vec::new();
        let mut tax_total = Money::ZERO;
        for (i, (id, base)) in taxable.iter().enumerate() {
            let code = codes
                .iter()
                .find(|c| c.id == *id)
                .ok_or(PayablesError::TaxCodeNotFound)?;
            let amount = match stated.get(id) {
                Some(a) => *a,
                None => {
                    let computed = (base.amount() * code.rate / Decimal::from(100))
                        .round_dp_with_strategy(
                            u32::from(book.minor_units.get()),
                            RoundingStrategy::MidpointAwayFromZero,
                        );
                    Money::new(computed, book.minor_units)
                        .map_err(|e| invalid("tax_lines", e.to_string()))?
                }
            };
            tax_total = tax_total + amount;
            taxes.push(json!({"tenant_id":tenant,"company_id":company,"line_number":i+1,"tax_code_id":id,"tax_code":code.code,"rate":code.rate,"account_id":code.account_id,"taxable_amount":base,"tax_amount":amount}));
        }
        let total = checked(net + tax_total, book.minor_units, "total", true)?;
        if let Some(b) = &original {
            if total
                > balances::lowest(tx.sql(), tenant, company, b.id, input.document_date).await?
            {
                return Err(PayablesError::ExceedsOutstanding);
            }
        }
        let kind = if bill.is_some() { "debit_note" } else { "bill" };
        let header = json!({"tenant_id":tenant,"company_id":company,"document_type":kind,"status":"draft","corrects_document_id":bill,"party_key":key,"party_name":name,"party_tax_id":tax_id,"party_registration_id":registration,"party_address":address,"party_document_number":number,"document_date":input.document_date.to_string(),"due_date":due.to_string(),"currency_code":book.currency_code,"description":description,"total_net":net,"total_tax":tax_total,"total":total,"created_by_user_id":actor,"created_by_impersonator_id":imp});
        let document = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let (row, change) = store::insert(sql, Table::Documents, header).await?;
                    let id = row["id"].as_i64().ok_or(Error::Invalid)?;
                    let mut changes = vec![change];
                    for mut row in lines {
                        row["document_id"] = json!(id);
                        changes.push(store::insert(sql, Table::Lines, row).await?.1);
                    }
                    for mut row in taxes {
                        row["document_id"] = json!(id);
                        changes.push(store::insert(sql, Table::TaxLines, row).await?.1);
                    }
                    let result = store::document(sql, tenant, company, id)
                        .await?
                        .ok_or(Error::NotFound)?;
                    Ok((result, changes))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(document)
    }
    pub async fn approve_document(&self, s: &Scope, company: i64, id: i64) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                APPROVE_DOCUMENTS,
                "finance.payables.approve_document",
            )
            .await?;
        let d = store::document(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)?;
        if d.status != "draft" {
            return Err(PayablesError::NotDraft);
        }
        self.ledger
            .check_approval(
                &mut tx,
                s,
                company,
                &Approval {
                    source: MODULE.into(),
                    action: "document_approval".into(),
                    subject_type: d.document_type.clone(),
                    subject_id: id,
                    subject: d.label(),
                    prepared_by: vec![Preparer {
                        user_id: d.created_by_user_id,
                        impersonator_id: d.created_by_impersonator_id,
                    }],
                    override_reason: None,
                },
            )
            .await?;
        let result = advance(&mut tx, s, company, id, "approved", None, None).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn void_document(
        &self,
        s: &Scope,
        company: i64,
        id: i64,
        reason: &str,
    ) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                APPROVE_DOCUMENTS,
                "finance.payables.void_document",
            )
            .await?;
        let d = store::document(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)?;
        if !matches!(d.status.as_str(), "draft" | "approved" | "held") {
            return Err(PayablesError::NotVoidable);
        }
        let reason = string(reason, "void_reason", 500, true)?;
        let result = advance(&mut tx, s, company, id, "voided", reason, None).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn post_document(&self, s: &Scope, company: i64, id: i64) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                POST_DOCUMENTS,
                "finance.payables.post_document",
            )
            .await?;
        let d = store::document(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)?;
        if !matches!(d.status.as_str(), "approved" | "held") {
            return Err(PayablesError::NotApproved);
        }
        let actor = (s.user(), s.impersonator().map(|p| p.user()));
        let mut matched = None;
        if d.document_type == "bill" && d.lines.iter().any(|l| l.order_line_id.is_some()) {
            let settings = store::settings(tx.sql(), s.tenant(), company)
                .await?
                .ok_or(PayablesError::PayablesNotConfigured)?
                .1;
            let book = self.ledger.get_book_on(tx.sql(), s, company).await?;
            let lines = matching::evaluate(
                tx.sql(),
                s.tenant(),
                company,
                &d,
                &settings,
                &book.currency_code,
                self.order_source,
            )
            .await?;
            if !lines.iter().all(|l| l.exceptions.is_empty())
                && !matching::covered(&approved_lines(&d), &lines)
            {
                return self.hold(tx, s, company, d, actor, lines).await;
            }
            matched = Some(lines);
        }
        let payable = if let Some(bill) = d.corrects_document_id {
            let b = store::document(tx.sql(), s.tenant(), company, bill)
                .await?
                .ok_or(PayablesError::DocumentNotFound)?;
            if d.total
                > balances::lowest(tx.sql(), s.tenant(), company, bill, d.document_date).await?
            {
                return Err(PayablesError::ExceedsOutstanding);
            }
            let named: Vec<_> = d
                .lines
                .iter()
                .map(|l| (l.line_number as usize, l.order_line_id, l.quantity))
                .collect();
            if let Err(e) =
                matching::within_billed(tx.sql(), s.tenant(), company, &b, &named, Some(id)).await
            {
                return Err(match e {
                    PayablesError::Invalid(_) => PayablesError::ExceedsBilledQuantity,
                    e => e,
                });
            }
            b.payable_account_id.ok_or(PayablesError::NotPostedBill)?
        } else {
            store::settings(tx.sql(), s.tenant(), company)
                .await?
                .ok_or(PayablesError::PayablesNotConfigured)?
                .1
                .payable_account_id
        };
        let debit = d.document_type == "bill";
        let side = |id, amount| {
            if debit {
                NewLine::debit(id, amount)
            } else {
                NewLine::credit(id, amount)
            }
        };
        let mut lines: Vec<NewLine> = d
            .lines
            .iter()
            .map(|l| side(l.account_id, l.amount).described(&l.description))
            .collect();
        lines.extend(
            d.tax_lines
                .iter()
                .filter(|t| !t.tax_amount.is_zero())
                .map(|t| side(t.account_id, t.tax_amount).described(&t.tax_code)),
        );
        lines.push(if debit {
            NewLine::credit(payable, d.total)
        } else {
            NewLine::debit(payable, d.total)
        });
        let description = if debit {
            format!(
                "Bill {} from {}",
                d.party_document_number.as_deref().unwrap_or_default(),
                d.party.name
            )
        } else {
            format!("Debit note to {}", d.party.name)
        };
        let entry = self
            .ledger
            .post_entry_in(
                &mut tx,
                s,
                company,
                &NewEntry {
                    entry_date: d.document_date,
                    description: description.chars().take(500).collect(),
                    reference: d.party_document_number.clone(),
                    lines,
                },
                &PostOptions {
                    owner: Some(EntryOwner {
                        module: MODULE.into(),
                        record: format!("document:{id}"),
                    }),
                    settlement_account_ids: if debit { vec![] } else { vec![payable] },
                },
            )
            .await?;
        if let Some(lines) = matched {
            let (tenant, document) = (s.tenant(), d.clone());
            tx.mutate(move |sql| {
                Box::pin(async move {
                    let changes = store::record_match(
                        sql,
                        &document,
                        (tenant, company),
                        actor,
                        "posted",
                        None,
                        &lines,
                    )
                    .await?;
                    Ok(((), changes))
                })
            })
            .await?;
        }
        let result = advance(
            &mut tx,
            s,
            company,
            id,
            "posted",
            None,
            Some((payable, entry.id, entry.entry_number)),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Records the hold and keeps (or makes) the bill `held`; nothing posts.
    async fn hold(
        &self,
        mut tx: MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        d: Document,
        actor: (i64, Option<i64>),
        lines: Vec<MatchLine>,
    ) -> Result<Document> {
        let (tenant, id, held) = (s.tenant(), d.id, d.status == "held");
        let document = d.clone();
        tx.mutate(move |sql| {
            Box::pin(async move {
                let changes = store::record_match(
                    sql,
                    &document,
                    (tenant, company),
                    actor,
                    "held",
                    None,
                    &lines,
                )
                .await?;
                Ok(((), changes))
            })
        })
        .await?;
        let result = if held {
            store::document(tx.sql(), tenant, company, id)
                .await?
                .ok_or(PayablesError::DocumentNotFound)?
        } else {
            advance(&mut tx, s, company, id, "held", None, None).await?
        };
        tx.commit().await?;
        Ok(result)
    }
    /// Approves a held bill's match exceptions with a reason and the hold the
    /// approver saw, so the next posting may post over them. The ledger's
    /// separation of duties applies (the bill's recorder and whoever tried to
    /// post it into the hold are its preparers), and the approval is always
    /// listed in its approval exceptions as an override.
    pub async fn approve_match_exception(
        &self,
        s: &Scope,
        company: i64,
        id: i64,
        approval: &MatchApproval,
    ) -> Result<Document> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                APPROVE_MATCH,
                "finance.payables.approve_match_exception",
            )
            .await?;
        let d = store::document(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)?;
        if d.status != "held" {
            return Err(PayablesError::NotHeld);
        }
        let held = match d.match_events.last() {
            Some(e) if e.event == "held" => e.clone(),
            _ => return Err(PayablesError::AlreadyApproved),
        };
        if approval.held_event_id != held.id {
            return Err(PayablesError::StaleHold);
        }
        let reason =
            string(&approval.reason, "reason", 500, true)?.ok_or(PayablesError::InvalidOption)?;
        self.ledger
            .check_approval(
                &mut tx,
                s,
                company,
                &Approval {
                    source: MODULE.into(),
                    action: "match_override".into(),
                    subject_type: "bill".into(),
                    subject_id: id,
                    subject: d.label(),
                    prepared_by: vec![
                        Preparer {
                            user_id: d.created_by_user_id,
                            impersonator_id: d.created_by_impersonator_id,
                        },
                        Preparer {
                            user_id: held.actor_user_id,
                            impersonator_id: held.actor_impersonator_id,
                        },
                    ],
                    override_reason: Some(reason.clone()),
                },
            )
            .await?;
        let (tenant, actor) = (s.tenant(), (s.user(), s.impersonator().map(|p| p.user())));
        let document = d.clone();
        tx.mutate(move |sql| {
            Box::pin(async move {
                let changes = store::record_match(
                    sql,
                    &document,
                    (tenant, company),
                    actor,
                    "exception_approved",
                    Some((&reason, held.id)),
                    &[],
                )
                .await?;
                Ok(((), changes))
            })
        })
        .await?;
        let result = store::document(tx.sql(), tenant, company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)?;
        tx.commit().await?;
        Ok(result)
    }
    /// The quantity billed against each of the order lines, by ID: posted
    /// bill lines naming it less posted debit note lines, in the company. An
    /// order line nothing posted names is absent. This is this module's own
    /// record, the figure the match takes off what was received.
    pub async fn billed_quantities(
        &self,
        s: &Scope,
        company: i64,
        order_line_ids: &[i64],
    ) -> Result<std::collections::BTreeMap<i64, Decimal>> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        if order_line_ids.iter().any(|id| *id < 1) || order_line_ids.len() > 500 {
            return Err(PayablesError::InvalidOption);
        }
        matching::billed_quantities(&**c, s.tenant(), company, order_line_ids, None).await
    }
    pub async fn get_document(&self, s: &Scope, company: i64, id: i64) -> Result<Document> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.get_document_on(&**c, s, company, id).await
    }
    /// Read and authorize on a sibling's transaction, without taking another
    /// pool connection while the caller holds the company write lock.
    pub async fn get_document_on(
        &self,
        c: &impl GenericClient,
        s: &Scope,
        company: i64,
        id: i64,
    ) -> Result<Document> {
        authorize(c, s, company, VIEW).await?;
        store::document(c, s.tenant(), company, id)
            .await?
            .ok_or(PayablesError::DocumentNotFound)
    }
    pub async fn list_documents(
        &self,
        s: &Scope,
        company: i64,
        q: &DocumentQuery,
    ) -> Result<Vec<Document>> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        filters(&q.document_type, &q.status)?;
        if !(1..=500).contains(&q.limit)
            || q.after.is_some_and(|(_, id)| id < 1)
            || matches!((q.from,q.to),(Some(f),Some(t)) if f>t)
        {
            return Err(PayablesError::InvalidOption);
        }
        let from = q.from.map(|d| d.to_string());
        let to = q.to.map(|d| d.to_string());
        let date = q.after.map(|(d, _)| d.to_string());
        let id = q.after.map(|(_, id)| id);
        let ids=c.query("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND ($3::text IS NULL OR document_type=$3) AND ($4::text IS NULL OR status=$4) AND ($5::text IS NULL OR party_key=$5) AND ($6::text IS NULL OR document_date>=$6::date) AND ($7::text IS NULL OR document_date<=$7::date) AND ($8::text IS NULL OR (document_date,id)>($8::date,$9::bigint)) ORDER BY document_date,id LIMIT $10",&[&s.tenant(),&company,&q.document_type,&q.status,&q.party_key,&from,&to,&date,&id,&q.limit]).await?;
        let mut docs = Vec::new();
        for id in ids {
            docs.push(
                store::document(&**c, s.tenant(), company, id.get(0))
                    .await?
                    .ok_or(PayablesError::DocumentNotFound)?,
            );
        }
        Ok(docs)
    }
    pub async fn page_documents(
        &self,
        s: &Scope,
        company: i64,
        q: &DocumentPageQuery,
    ) -> Result<DocumentPage> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        filters(&q.document_type, &q.status)?;
        if q.page < 1 || ![25, 50, 100].contains(&q.page_size) || q.search.chars().count() > 200 {
            return Err(PayablesError::InvalidOption);
        }
        let offset = (q.page - 1)
            .checked_mul(q.page_size)
            .ok_or(PayablesError::InvalidOption)?;
        let search = q.search.trim();
        let rows=c.query("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND ($3::text IS NULL OR document_type=$3) AND ($4::text IS NULL OR status=$4) AND (strpos(lower(party_name),lower($5))>0 OR strpos(COALESCE(document_number::text,''),$5)>0) ORDER BY document_date DESC,id DESC LIMIT $6 OFFSET $7",&[&s.tenant(),&company,&q.document_type,&q.status,&search,&q.page_size,&offset]).await?;
        let total=c.query_one("SELECT count(*) FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND ($3::text IS NULL OR document_type=$3) AND ($4::text IS NULL OR status=$4) AND (strpos(lower(party_name),lower($5))>0 OR strpos(COALESCE(document_number::text,''),$5)>0)",&[&s.tenant(),&company,&q.document_type,&q.status,&search]).await?.get(0);
        let mut documents = Vec::new();
        for row in rows {
            documents.push(
                store::document(&**c, s.tenant(), company, row.get(0))
                    .await?
                    .ok_or(PayablesError::DocumentNotFound)?,
            );
        }
        Ok(DocumentPage {
            documents,
            total,
            page: q.page,
            page_size: q.page_size,
        })
    }
}
async fn advance(
    tx: &mut MutationTransaction<'_>,
    s: &Scope,
    company: i64,
    id: i64,
    status: &'static str,
    reason: Option<String>,
    posting: Option<(i64, i64, i64)>,
) -> Result<Document> {
    let tenant = s.tenant();
    let actor = s.user();
    let imp = s.impersonator().map(|p| p.user());
    Ok(tx.mutate(move |sql|Box::pin(async move {
        let before=store::raw(sql,Table::Documents,tenant,company,id).await?;
        match status {
            "approved"=> {sql.execute("UPDATE finance_payables_documents SET status='approved',approved_by_user_id=$4,approved_by_impersonator_id=$5 WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id,&actor,&imp]).await?;},
            "held"=> {sql.execute("UPDATE finance_payables_documents SET status='held' WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id]).await?;},
            "voided"=> {sql.execute("UPDATE finance_payables_documents SET status='voided',voided_by_user_id=$4,voided_by_impersonator_id=$5,void_reason=$6 WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id,&actor,&imp,&reason]).await?;},
            "posted"=> {let (account,entry,number)=posting.ok_or(Error::Invalid)?;sql.execute("UPDATE finance_payables_documents SET status='posted',posted_by_user_id=$4,posted_by_impersonator_id=$5,payable_account_id=$6,entry_id=$7,entry_number=$8 WHERE tenant_id=$1 AND company_id=$2 AND id=$3",&[&tenant,&company,&id,&actor,&imp,&account,&entry,&number]).await?;},
            _=>return Err(Error::Invalid),
        }
        let after=store::raw(sql,Table::Documents,tenant,company,id).await?;
        let d=store::document(sql,tenant,company,id).await?.ok_or(Error::NotFound)?;
        Ok((d,vec![store::update_change(Table::Documents,id,before,after)]))
    })).await?)
}

/// The lines of the hold the bill's latest approval covers, with the figures
/// the approver saw, or none when the latest event is not an approval.
fn approved_lines(d: &Document) -> Vec<MatchLine> {
    match d.match_events.last() {
        Some(e) if e.event == "exception_approved" => d
            .match_events
            .iter()
            .find(|h| Some(h.id) == e.held_event_id)
            .map(|h| h.lines.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}
