//! Conversion bank items are evidence of amounts already in the opening
//! journal, never additional postings. This module owns their source and bridge.
use crate::*;
use bilibi_agent_api::OperationClient;
use bilibi_database::Change;
use bilibi_finance_ledger::{EntryOwner, Money, OpeningContext};
use bilibi_party_ids::OrganisationId;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const HEADER: &[&str] = &["source_id", "item_date", "description", "amount"];

fn invalid(field: &'static str, message: impl Into<String>) -> ReconciliationError {
    ReconciliationError::Invalid(vec![FieldError::new(field, message)])
}

pub(super) struct Prepared {
    pub review: Value,
    context: OpeningContext,
    account_id: i64,
    ledger_account_id: i64,
    balance: Money,
    statement: Money,
    rows: Vec<Value>,
    existing: Option<Value>,
}

impl Reconciliation {
    async fn authorize_on(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        capability: &str,
    ) -> Result<()> {
        self.parties
            .organisation_target(
                c,
                s,
                OrganisationId::new(company).map_err(|_| ReconciliationError::CompanyNotFound)?,
                capability,
            )
            .await
            .map(|_| ())
            .map_err(refusal)
    }

    pub(super) async fn opening_context(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.authorize_on(c, s, company, VIEW).await?;
        let account = input["bank_account_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or_else(|| invalid("bank_account_id", "choose a bank account"))?;
        let row = c.query_opt("SELECT id,name,ledger_account_id,active FROM finance_reconciliation_bank_accounts WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&s.tenant(), &company, &account]).await?.ok_or(ReconciliationError::BankAccountNotFound)?;
        let opening = self.ledger.opening_context_on(c, s, company).await?;
        let book = self.ledger.get_book_on(c, s, company).await?;
        let bridge = c.query_opt("SELECT to_jsonb(b) FROM finance_reconciliation_opening_bridges b WHERE tenant_id=$1 AND company_id=$2 AND bank_account_id=$3 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges r WHERE r.replaces_bridge_id=b.id) ORDER BY id DESC LIMIT 1", &[&s.tenant(), &company, &account]).await?.map(|r| r.get::<_, Value>(0));
        let items = if let Some(id) = bridge.as_ref().and_then(|b| b["id"].as_i64()) {
            c.query("SELECT to_jsonb(i) FROM finance_reconciliation_opening_items i WHERE tenant_id=$1 AND company_id=$2 AND bridge_id=$3 ORDER BY id", &[&s.tenant(), &company, &id]).await?.iter().map(|r| r.get::<_, Value>(0)).collect::<Vec<_>>()
        } else {
            vec![]
        };
        Ok(
            json!({"account":{"id":account,"name":row.get::<_,String>(1),"ledger_account_id":row.get::<_,i64>(2),"active":row.get::<_,bool>(3)}, "opening":opening,"book":book,"bridge":bridge,"items":items,"header":HEADER}),
        )
    }

    pub(super) async fn prepare_opening(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Prepared> {
        self.authorize_on(c, s, company, MANAGE_RECONCILIATIONS)
            .await?;
        let tenant = s.tenant();
        let company_row = c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE NOWAIT", &[&tenant, &company]).await?.ok_or(ReconciliationError::CompanyNotFound)?;
        if company_row.get::<_, String>(0) == "archived" {
            return Err(ReconciliationError::CompanyArchived);
        }
        let account_id = input["bank_account_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or_else(|| invalid("bank_account_id", "choose a bank account"))?;
        let account = c.query_opt("SELECT name,ledger_account_id,active FROM finance_reconciliation_bank_accounts WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&tenant, &company, &account_id]).await?.ok_or(ReconciliationError::BankAccountNotFound)?;
        if !account.get::<_, bool>(2) {
            return Err(ReconciliationError::BankAccountInactive);
        }
        let ledger_account_id: i64 = account.get(1);
        let context = self.ledger.opening_context_on(c, s, company).await?;
        let book = self.ledger.get_book_on(c, s, company).await?;
        let balance = context
            .balances
            .get(&ledger_account_id)
            .copied()
            .ok_or_else(|| {
                invalid(
                    "bank_account_id",
                    "must be included in the approved opening balances",
                )
            })?;
        if input["currency_code"] != book.currency_code {
            return Err(invalid("currency_code", "must match the book's currency"));
        }
        let reference = input["source_reference"].as_str().unwrap_or("").trim();
        if reference.is_empty() || reference.chars().count() > 100 || reference.contains('\0') {
            return Err(invalid(
                "source_reference",
                "must contain 1 to 100 characters without null characters",
            ));
        }
        let statement_source = input["statement_source"].as_str().unwrap_or("").trim();
        if statement_source.is_empty()
            || statement_source.chars().count() > 500
            || statement_source.contains('\0')
        {
            return Err(invalid(
                "statement_source",
                "identify the source statement in 1 to 500 characters",
            ));
        }
        let csv = input["csv"]
            .as_str()
            .ok_or_else(|| invalid("csv", "is required"))?;
        if csv.len() > 131072 || csv.contains('\0') {
            return Err(invalid(
                "csv",
                "must fit within 128 KiB and contain no null characters",
            ));
        }
        let statement = Money::parse(
            input["statement_balance"].as_str().unwrap_or(""),
            book.minor_units,
        )
        .map_err(|e| invalid("statement_balance", e.to_string()))?;
        let replaces = input["replaces_bridge_id"].as_i64();
        let reason = input["reason"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if replaces.is_some() != reason.is_some()
            || reason.is_some_and(|r| r.chars().count() > 500 || r.contains('\0'))
        {
            return Err(invalid(
                "reason",
                "a correction needs a reason of at most 500 characters",
            ));
        }
        let existing = c.query_opt("SELECT to_jsonb(b) FROM finance_reconciliation_opening_bridges b WHERE tenant_id=$1 AND company_id=$2 AND source_reference=$3", &[&tenant, &company, &reference]).await?.map(|r| r.get::<_,Value>(0));
        if let Some(batch) = existing {
            let existing_amount = batch["statement_balance"]
                .to_string()
                .trim_matches('"')
                .to_owned();
            if batch["bank_account_id"] != account_id
                || batch["source_csv"] != csv
                || batch["statement_source"] != statement_source
                || batch["replaces_bridge_id"].as_i64() != replaces
                || batch["reason"].as_str() != reason
                || Money::parse(&existing_amount, book.minor_units).ok() != Some(statement)
            {
                return Err(invalid(
                    "source_reference",
                    "already identifies different opening evidence",
                ));
            }
            return Ok(Prepared {
                review: json!({"already_imported":true,"bridge":batch,"posts":false}),
                context,
                account_id,
                ledger_account_id,
                balance,
                statement,
                rows: vec![],
                existing: Some(batch),
            });
        }
        let current = c.query_opt("SELECT id FROM finance_reconciliation_opening_bridges b WHERE tenant_id=$1 AND company_id=$2 AND bank_account_id=$3 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges r WHERE r.replaces_bridge_id=b.id)", &[&tenant, &company, &account_id]).await?.map(|r| r.get::<_,i64>(0));
        if current != replaces {
            return Err(invalid(
                "replaces_bridge_id",
                "a correction must name the current bridge; an account has only one opening bridge",
            ));
        }
        if c.query_opt("SELECT r.id FROM finance_reconciliation_reconciliations r JOIN finance_reconciliation_reconciliation_events e ON e.reconciliation_id=r.id AND e.action='finalised' WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.bank_account_id=$3 LIMIT 1", &[&tenant,&company,&account_id]).await?.is_some() { return Err(invalid("bank_account_id", "a finalised reconciliation prevents changing the opening bridge")); }
        if c.query_opt("SELECT m.id FROM finance_reconciliation_matches m JOIN finance_reconciliation_match_events e ON e.match_id=m.id AND e.action='matched' WHERE m.tenant_id=$1 AND m.company_id=$2 AND m.bank_account_id=$3 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events u WHERE u.match_id=m.id AND u.action='unmatched') LIMIT 1", &[&tenant,&company,&account_id]).await?.is_some() { return Err(invalid("bank_account_id", "unmatch the account before recording or correcting its opening bridge")); }
        if c.query_opt("SELECT id FROM finance_reconciliation_statement_lines WHERE tenant_id=$1 AND company_id=$2 AND bank_account_id=$3 AND line_date <= $4::text::date LIMIT 1", &[&tenant,&company,&account_id,&context.entry.entry_date.to_string()]).await?.is_some() { return Err(invalid("bank_account_id", "opening conversion must precede statement lines dated on or before the conversion date")); }
        let parsed = csv::parse(csv.as_bytes(), ",")
            .map_err(|e| invalid("csv", format!("cannot read CSV: {e:?}")))?;
        if parsed
            .first()
            .map(|r| r.iter().map(String::as_str).collect::<Vec<_>>())
            != Some(HEADER.to_vec())
            || parsed.len() > 501
        {
            return Err(invalid(
                "csv",
                "requires header source_id,item_date,description,amount and at most 500 rows",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        let mut total = Money::ZERO;
        for (n, row) in parsed.iter().enumerate().skip(1) {
            let err = |text: &str| invalid("csv", format!("row {}: {text}", n + 1));
            if row.len() != 4 {
                return Err(err("must contain four columns"));
            }
            let id = row[0].trim();
            let description = row[2].trim();
            if id.is_empty() || id.chars().count() > 200 || !seen.insert(id.to_owned()) {
                return Err(err(
                    "source_id must be unique and contain 1 to 200 characters",
                ));
            }
            if description.is_empty() || description.chars().count() > 500 {
                return Err(err("description must contain 1 to 500 characters"));
            }
            let date: jiff::civil::Date = row[1]
                .trim()
                .parse()
                .map_err(|_| err("item_date must be YYYY-MM-DD"))?;
            if date > context.entry.entry_date {
                return Err(err("item_date must not be after the conversion date"));
            }
            let amount = Money::parse(&row[3], book.minor_units)
                .map_err(|_| err("amount must use the book's currency precision"))?;
            if amount.is_zero() {
                return Err(err("amount must not be zero"));
            }
            total = total + amount;
            rows.push(json!({"source_id":id,"item_date":date.to_string(),"description":description,"amount":amount}));
        }
        let difference = statement - balance + total;
        if !difference.is_zero() {
            return Err(invalid("csv", format!("statement balance less opening book balance plus outstanding items is {}; check the source statement and items", difference.display(book.minor_units))));
        }
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
            review: json!({"already_imported":false,"source_reference":reference,"statement_source":statement_source,"bank_account_id":account_id,"bank_account_name":account.get::<_,String>(0),"ledger_account_id":ledger_account_id,"source_trial_balance_reference":context.source_reference,"conversion_date":context.entry.entry_date.to_string(),"entry_id":context.entry.id,"entry_number":context.entry.entry_number,"currency_code":book.currency_code,"minor_units":book.minor_units,"source_trial_balance":balance,"opening_bank_balance":balance,"statement_balance":statement,"items":rows,"items_total":total,"difference":difference,"posts":false,"replaces_bridge_id":replaces,"reason":reason,"approval":approval}),
            context,
            account_id,
            ledger_account_id,
            balance,
            statement,
            rows,
            existing: None,
        })
    }

    pub(super) async fn prepare_opening_match(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.authorize_on(c, s, company, MANAGE_MATCHES).await?;
        let tenant = s.tenant();
        let company_row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE NOWAIT", &[&tenant,&company]).await?.ok_or(ReconciliationError::CompanyNotFound)?;
        if company_row.get::<_, String>(0) == "archived" {
            return Err(ReconciliationError::CompanyArchived);
        }
        let account = input["bank_account_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or(ReconciliationError::InvalidOption)?;
        let item = input["opening_item_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or(ReconciliationError::InvalidOption)?;
        let line = input["statement_line_id"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or(ReconciliationError::InvalidOption)?;
        let item_row=c.query_opt("SELECT i.source_id,to_char(i.item_date,'YYYY-MM-DD'),i.description,i.amount,to_char(b.conversion_date,'YYYY-MM-DD') FROM finance_reconciliation_opening_items i JOIN finance_reconciliation_opening_bridges b ON b.id=i.bridge_id WHERE i.tenant_id=$1 AND i.company_id=$2 AND i.id=$3 AND b.bank_account_id=$4 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges r WHERE r.replaces_bridge_id=b.id)", &[&tenant,&company,&item,&account]).await?.ok_or_else(|| invalid("opening_item_id", "choose a current opening item for this account"))?;
        let line_row=c.query_opt("SELECT to_char(line_date,'YYYY-MM-DD'),description,amount FROM finance_reconciliation_statement_lines WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND bank_account_id=$4", &[&tenant,&company,&line,&account]).await?.ok_or(ReconciliationError::StatementLineNotFound)?;
        if c.query_opt("SELECT x.id FROM finance_reconciliation_match_lines x JOIN finance_reconciliation_match_events e ON e.match_id=x.match_id AND e.action='matched' WHERE x.tenant_id=$1 AND x.company_id=$2 AND x.statement_line_id=$3 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events u WHERE u.match_id=x.match_id AND u.action='unmatched')", &[&tenant,&company,&line]).await?.is_some() { return Err(ReconciliationError::StatementLineMatched); }
        if c.query_opt("SELECT x.id FROM finance_reconciliation_match_opening_items x JOIN finance_reconciliation_match_events e ON e.match_id=x.match_id AND e.action='matched' WHERE x.tenant_id=$1 AND x.company_id=$2 AND x.opening_item_id=$3 AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events u WHERE u.match_id=x.match_id AND u.action='unmatched')", &[&tenant,&company,&item]).await?.is_some() { return Err(invalid("opening_item_id", "is already matched")); }
        let source_amount = statements::stored(item_row.get(3))?;
        let line_amount = statements::stored(line_row.get(2))?;
        if source_amount != line_amount {
            return Err(ReconciliationError::Unbalanced {
                statement_side: line_amount,
                ledger_side: source_amount,
            });
        }
        let line_date: String = line_row.get(0);
        let conversion_date: String = item_row.get(4);
        if line_date <= conversion_date {
            return Err(invalid(
                "statement_line_id",
                "must be after the conversion date",
            ));
        }
        Ok(
            json!({"bank_account_id":account,"opening_item_id":item,"source_id":item_row.get::<_,String>(0),"item_date":item_row.get::<_,String>(1),"item_description":item_row.get::<_,String>(2),"amount":source_amount,"statement_line_id":line,"statement_date":line_date,"statement_description":line_row.get::<_,String>(1),"posts":false}),
        )
    }

    pub(super) async fn match_opening_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Match> {
        let review = self
            .prepare_opening_match(tx.sql(), s, company, input)
            .await?;
        let account = review["bank_account_id"].as_i64().ok_or(Error::Internal)?;
        let line_id = review["statement_line_id"]
            .as_i64()
            .ok_or(Error::Internal)?;
        let item_id = review["opening_item_id"].as_i64().ok_or(Error::Internal)?;
        let line = matching::find_line(tx.sql(), s.tenant(), company, line_id)
            .await?
            .ok_or(ReconciliationError::StatementLineNotFound)?;
        let id = matching::insert(
            tx,
            s.tenant(),
            company,
            people(s),
            account,
            &[line],
            &[],
            Some("Opening item".into()),
        )
        .await?;
        let tenant = s.tenant();
        tx.mutate(move |sql| Box::pin(async move {
            let row=sql.query_one("INSERT INTO finance_reconciliation_match_opening_items (tenant_id,company_id,match_id,opening_item_id) VALUES ($1,$2,$3,$4) RETURNING id", &[&tenant,&company,&id,&item_id]).await?;
            Ok(((),vec![Change {kind:"finance.reconciliation.match_opening_item".into(),id:row.get(0),old:Value::Null,new:json!({"company_id":company,"match_id":id,"opening_item_id":item_id})}]))
        })).await?;
        matching::find(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or_else(|| Error::Internal.into())
    }

    pub(super) async fn prepare_opening_unmatch(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        self.authorize_on(c, s, company, MANAGE_MATCHES).await?;
        let tenant = s.tenant();
        let row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE NOWAIT", &[&tenant,&company]).await?.ok_or(ReconciliationError::CompanyNotFound)?;
        if row.get::<_, String>(0) == "archived" {
            return Err(ReconciliationError::CompanyArchived);
        }
        let id = input["match_id"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or(ReconciliationError::InvalidOption)?;
        let reason = matching::unmatch_reason(input["reason"].as_str())
            .map_err(|e| ReconciliationError::Invalid(vec![e]))?;
        let row=c.query_opt("SELECT m.bank_account_id,m.match_number,e.event_number FROM finance_reconciliation_matches m JOIN finance_reconciliation_match_events e ON e.match_id=m.id AND e.action='matched' WHERE m.tenant_id=$1 AND m.company_id=$2 AND m.id=$3 AND EXISTS (SELECT 1 FROM finance_reconciliation_match_opening_items x WHERE x.match_id=m.id)", &[&tenant,&company,&id]).await?.ok_or(ReconciliationError::MatchNotFound)?;
        if c.query_opt("SELECT id FROM finance_reconciliation_match_events WHERE tenant_id=$1 AND company_id=$2 AND match_id=$3 AND action='unmatched'", &[&tenant,&company,&id]).await?.is_some() {return Err(ReconciliationError::MatchNotStanding);}
        let account: i64 = row.get(0);
        let number: i64 = row.get(1);
        let matched: i64 = row.get(2);
        if c.query_opt("SELECT e.id FROM finance_reconciliation_reconciliation_events e JOIN finance_reconciliation_reconciliations r ON r.id=e.reconciliation_id WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.bank_account_id=$3 AND e.action='finalised' AND e.through_match_event_number >= $4 LIMIT 1", &[&tenant,&company,&account,&matched]).await?.is_some() {return Err(ReconciliationError::MatchFinalised);}
        Ok(
            json!({"match_id":id,"match_number":number,"bank_account_id":account,"reason":reason,"posts":false}),
        )
    }
    pub(super) async fn unmatch_opening_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Match> {
        let review = self
            .prepare_opening_unmatch(tx.sql(), s, company, input)
            .await?;
        let id = review["match_id"].as_i64().ok_or(Error::Internal)?;
        let reason = review["reason"].as_str().ok_or(Error::Internal)?.to_owned();
        matching::record_unmatch(tx, s.tenant(), company, people(s), id, reason).await?;
        matching::find(tx.sql(), s.tenant(), company, id)
            .await?
            .ok_or_else(|| Error::Internal.into())
    }

    pub(super) async fn import_opening_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        input: &Value,
    ) -> Result<Value> {
        let p = self.prepare_opening(tx.sql(), s, company, input).await?;
        if let Some(existing) = p.existing {
            return Ok(existing);
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
        let (actor, impersonator) = people(s);
        let source = input["source_reference"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let statement_source = input["statement_source"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned();
        let csv = input["csv"].as_str().unwrap_or("").to_owned();
        let replaces = input["replaces_bridge_id"].as_i64();
        let reason = input["reason"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let rows = p.rows;
        let conversion = p.context.entry.entry_date.to_string();
        let entry_id = p.context.entry.id;
        let entry_number = p.context.entry.entry_number;
        let account_id = p.account_id;
        let statement = p.statement.amount();
        let balance = p.balance.amount();
        let review = p.review;
        let audit_review = review.clone();
        let batch = tx.mutate(move |sql| Box::pin(async move {
            let row=sql.query_one("INSERT INTO finance_reconciliation_opening_bridges (tenant_id,company_id,bank_account_id,source_reference,statement_source,source_csv,conversion_date,statement_balance,book_balance,opening_entry_id,opening_entry_number,item_count,replaces_bridge_id,reason,recorded_by_user_id,recorded_by_impersonator_id,review) VALUES ($1,$2,$3,$4,$5,$6,$7::text::date,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) RETURNING id", &[&tenant,&company,&account_id,&source,&statement_source,&csv,&conversion,&statement,&balance,&entry_id,&entry_number,&(rows.len() as i32),&replaces,&reason,&actor,&impersonator,&audit_review]).await?;
            let id:i64 = row.get(0);
            let mut changes = vec![Change {kind:"finance.reconciliation.opening_bridge".into(), id, old:Value::Null, new:json!({"company_id":company,"bank_account_id":account_id,"source_reference":source,"statement_source":statement_source,"source_csv":csv,"conversion_date":conversion,"statement_balance":statement.to_string(),"book_balance":balance.to_string(),"opening_entry_id":entry_id,"opening_entry_number":entry_number,"replaces_bridge_id":replaces,"reason":reason,"recorded_by_user_id":actor,"recorded_by_impersonator_id":impersonator,"review":audit_review})}];
            for item in rows {
                let amount = Money::parse(item["amount"].as_str().ok_or(Error::Invalid)?, MinorUnits::new(4).ok_or(Error::Internal)?).map_err(|_| Error::Invalid)?;
                let row=sql.query_one("INSERT INTO finance_reconciliation_opening_items (tenant_id,company_id,bridge_id,source_id,item_date,description,amount) VALUES ($1,$2,$3,$4,$5::text::date,$6,$7) RETURNING id", &[&tenant,&company,&id,&item["source_id"].as_str(),&item["item_date"].as_str(),&item["description"].as_str(),&amount.amount()]).await?;
                changes.push(Change{kind:"finance.reconciliation.opening_item".into(),id:row.get(0),old:Value::Null,new:json!({"company_id":company,"bridge_id":id,"source":item})});
            }
            Ok((id,changes))
        })).await?;
        self.ledger
            .replace_opening_recognition_on(
                tx,
                s,
                company,
                &EntryOwner {
                    module: MODULE.into(),
                    record: format!("opening:{batch}"),
                },
                replaces.map(|id| format!("opening:{id}")).as_deref(),
                p.ledger_account_id,
                p.balance,
            )
            .await?;
        Ok(json!({"id":batch,"review":review}))
    }
}
