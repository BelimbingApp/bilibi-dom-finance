//! NP3 running proof: Bilimbi Payables payment workflow and reproducible figures.
use crate::{sign_in, COMPANY};
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error};
use bilibi_finance_attachments::{AttachmentError, Attachments, NewFile};
use bilibi_finance_ledger::{
    AccountChange, AccountType, Ledger, LedgerError, Money, NewAccount, NewEntry, NewLine,
    PostOptions,
};
use bilibi_finance_payables::{
    AgingQuery, AllocationInput, Document, DocumentLineInput, NewDocument, NewPayment, NewTaxCode,
    Party, Payables, PayablesError, PaymentQuery, PaymentRequestQuery, Settings, SettingsInput,
    TaxCodeDetails, TaxQuery, PAYMENT_SOURCE,
};
use bilibi_module_registry::Services;
use bilibi_tenancy::{Authentication, Scope};
use jiff::civil::{date, Date};
use std::fmt::Debug;
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
fn refused<T: Debug>(
    label: &str,
    r: Result<T, PayablesError>,
    predicate: impl FnOnce(&PayablesError) -> bool,
) -> Outcome<()> {
    match r {
        Err(e) if predicate(&e) => {
            println!("{label}: refused ({e:?})");
            Ok(())
        }
        other => Err(format!("{label}: unexpected {other:?}").into()),
    }
}
async fn raw_refusal(c: &tokio_postgres::Client, label: &str, sql: &str) -> Outcome<()> {
    let result = c.batch_execute(sql).await;
    let _ = c.batch_execute("ROLLBACK").await;
    match result {
        Err(e) => {
            println!(
                "{label}: database refused ({})",
                e.as_db_error()
                    .map_or_else(|| e.to_string(), |e| e.message().into())
            );
            Ok(())
        }
        Ok(()) => Err(format!("{label}: database accepted forbidden write").into()),
    }
}
fn bill_input(number: &str, expense: i64, amount: Money) -> NewDocument {
    NewDocument {
        party: Party {
            key: "np3-party".into(),
            name: "Payment proof party".into(),
            ..Default::default()
        },
        party_document_number: Some(number.into()),
        document_date: date(2026, 12, 1),
        due_date: Some(date(2026, 12, 10)),
        description: None,
        lines: vec![DocumentLineInput {
            description: "Payment proof line".into(),
            account_id: expense,
            amount,
            tax_code_id: None,
            external_order_line_id: None,
            order_line_id: None,
            quantity: None,
        }],
        tax_lines: None,
    }
}
async fn post_bill(
    p: &Payables,
    writer: &Scope,
    reviewer: &Scope,
    input: &NewDocument,
) -> Outcome<Document> {
    let b = p.create_bill(writer, COMPANY, input).await?;
    p.approve_document(reviewer, COMPANY, b.id).await?;
    Ok(p.post_document(writer, COMPANY, b.id).await?)
}
fn request(bill: i64, bank: i64, amount: Money, date: Date) -> NewPayment {
    NewPayment {
        payment_date: date,
        party_key: "np3-party".into(),
        paid_from_account_id: bank,
        reference: Some("NP3 payment proof".into()),
        description: None,
        allocations: vec![AllocationInput {
            document_id: bill,
            amount,
        }],
    }
}
pub async fn run(db: &Db, services: &Services) -> Outcome<()> {
    println!("\n== Payables NP3: payment decisions, reversals and pinned reports");
    let p = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let attachments = services.get::<Attachments>(bilibi_finance_attachments::SERVICE)?;
    let manager = sign_in(db, services, 1, 1).await?;
    let reviewer = sign_in(db, services, 1, 4).await?;
    let staff = sign_in(db, services, 1, 3).await?;
    let foreign = sign_in(db, services, 2, 5).await?;
    let units = ledger.get_book(&manager, COMPANY).await?.minor_units;
    let money = |s| Money::parse(s, units).expect("fixture money");
    let payable = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("2197", "Payment proof control", AccountType::Liability),
        )
        .await?;
    let other_payable = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("2198", "Payment proof next control", AccountType::Liability),
        )
        .await?;
    let bank = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("1197", "Payment proof paid from", AccountType::Asset),
        )
        .await?;
    let expense = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("5197", "Payment proof expense", AccountType::Expense),
        )
        .await?;
    p.put_settings(
        &manager,
        COMPANY,
        &SettingsInput {
            settings: Settings {
                payable_account_id: payable.id,
                quantity_tolerance: None,
                price_tolerance: None,
            },
            reason: Some("NP3 proof".into()),
        },
    )
    .await?;
    let b = post_bill(
        &p,
        &manager,
        &reviewer,
        &bill_input("NP3-BILL", expense.id, money("100")),
    )
    .await?;
    let input = request(b.id, bank.id, money("40"), date(2026, 12, 15));
    refused(
        "staff cannot submit",
        p.submit_payment(&staff, COMPANY, &input).await,
        |e| matches!(e, PayablesError::Unauthorized),
    )?;
    refused(
        "foreign tenant cannot submit",
        p.submit_payment(&foreign, COMPANY, &input).await,
        |e| {
            matches!(
                e,
                PayablesError::CompanyNotFound | PayablesError::Unauthorized
            )
        },
    )?;
    let mut invalid = input.clone();
    invalid.allocations.clear();
    refused(
        "empty allocations",
        p.submit_payment(&manager, COMPANY, &invalid).await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    invalid = input.clone();
    invalid.allocations.push(invalid.allocations[0].clone());
    refused(
        "duplicate bill allocation",
        p.submit_payment(&manager, COMPANY, &invalid).await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    invalid = input.clone();
    invalid.party_key = "different-party".into();
    refused(
        "another party bill",
        p.submit_payment(&manager, COMPANY, &invalid).await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    invalid = input.clone();
    invalid.payment_date = date(2026, 11, 30);
    refused(
        "payment before bill",
        p.submit_payment(&manager, COMPANY, &invalid).await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    invalid = input.clone();
    invalid.allocations[0].amount = Money::parse(
        "0.001",
        bilibi_finance_ledger::MinorUnits::new(4).ok_or(Error::Invalid)?,
    )?;
    refused(
        "book precision",
        p.submit_payment(&manager, COMPANY, &invalid).await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    refused(
        "archived payment company",
        p.submit_payment(&reviewer, 3, &input).await,
        |e| matches!(e, PayablesError::CompanyArchived),
    )?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants
        .set_direct(
            &reviewer,
            1,
            bilibi_finance_payables::SUBMIT_PAYMENTS,
            Some(false),
        )
        .await?;
    refused(
        "revoked grant on stale scope",
        p.submit_payment(&manager, COMPANY, &input).await,
        |e| matches!(e, PayablesError::Unauthorized),
    )?;
    grants
        .set_direct(&reviewer, 1, bilibi_finance_payables::SUBMIT_PAYMENTS, None)
        .await?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let as_manager = {
        let c = db.get().await?;
        authentication
            .authenticate_session(&**c, 1, 1, Some(4), String::new())
            .await?
    };
    let impersonated = p.submit_payment(&as_manager, COMPANY, &input).await?;
    refused(
        "submitter's impersonator cannot approve",
        p.approve_payment(&reviewer, COMPANY, impersonated.id).await,
        |e| matches!(e, PayablesError::Ledger(LedgerError::SamePerson)),
    )?;
    p.reject_payment(
        &reviewer,
        COMPANY,
        impersonated.id,
        "Impersonation separation proof",
    )
    .await?;
    let outside = request(b.id, bank.id, money("1"), date(2027, 1, 1));
    let outside = p.submit_payment(&manager, COMPANY, &outside).await?;
    refused(
        "payment outside an open fiscal period",
        p.approve_payment(&reviewer, COMPANY, outside.id).await,
        |e| matches!(e, PayablesError::Ledger(LedgerError::Invalid(_))),
    )?;
    assert_eq!(
        p.get_payment_request(&manager, COMPANY, outside.id)
            .await?
            .status,
        "submitted"
    );
    p.reject_payment(&reviewer, COMPANY, outside.id, "No fiscal period yet")
        .await?;
    let r = p.submit_payment(&manager, COMPANY, &input).await?;
    assert_eq!(r.status, "submitted");
    assert_eq!(
        p.get_document(&manager, COMPANY, b.id).await?.outstanding,
        Some(money("100"))
    );
    refused(
        "same person payment approval",
        p.approve_payment(&manager, COMPANY, r.id).await,
        |e| matches!(e, PayablesError::Ledger(LedgerError::SamePerson)),
    )?;
    let file = NewFile {
        file_name: "payment-proof.txt".into(),
        content_type: Some("text/plain".into()),
        content: b"Payment proof".to_vec(),
    };
    let attachment = attachments
        .attach(&manager, COMPANY, PAYMENT_SOURCE, r.id, &file)
        .await?;
    let attachment = attachments
        .replace(&manager, COMPANY, attachment.id, &file)
        .await?;
    let paid = p.approve_payment(&reviewer, COMPANY, r.id).await?;
    assert!(matches!(
        attachments
            .replace(&manager, COMPANY, attachment.id, &file)
            .await,
        Err(AttachmentError::DocumentFinal)
    ));
    assert_eq!(paid.payment_number, 1);
    assert_eq!(paid.posted_by_user_id, reviewer.user());
    let decision = p.get_payment_request(&manager, COMPANY, r.id).await?;
    assert_eq!(decision.status, "posted");
    assert_eq!(
        decision.decision.ok_or(Error::NotFound)?.payment_id,
        Some(paid.id)
    );
    let entry = ledger.get_entry(&manager, COMPANY, paid.entry_id).await?;
    assert_eq!(
        entry.owner.ok_or(Error::NotFound)?.record,
        format!("payment:{}", paid.id)
    );
    refused(
        "decided request",
        p.approve_payment(&reviewer, COMPANY, r.id).await,
        |e| matches!(e, PayablesError::PaymentRequestDecided),
    )?;
    let pinned = p
        .aged_payables(
            &manager,
            COMPANY,
            date(2026, 12, 31),
            &AgingQuery::default(),
        )
        .await?;
    let party = pinned
        .parties
        .iter()
        .find(|p| p.party_key == "np3-party")
        .ok_or(Error::NotFound)?;
    assert_eq!(party.total, money("60"));
    assert_eq!(party.buckets[1], money("60"));
    let r2 = p
        .submit_payment(
            &manager,
            COMPANY,
            &request(b.id, bank.id, money("30"), date(2026, 12, 14)),
        )
        .await?;
    p.approve_payment(&reviewer, COMPANY, r2.id).await?;
    let reproduced = p
        .aged_payables(
            &manager,
            COMPANY,
            date(2026, 12, 31),
            &AgingQuery {
                through_entry_number: Some(pinned.through_entry_number),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(reproduced.total, pinned.total);
    assert_eq!(reproduced.buckets, pinned.buckets);
    refused(
        "reversal before original",
        p.reverse_payment(&manager, COMPANY, paid.id, date(2026, 12, 14), None)
            .await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    let reversal = p
        .reverse_payment(&manager, COMPANY, paid.id, date(2026, 12, 20), None)
        .await?;
    assert!(reversal.allocations.is_empty());
    assert_eq!(reversal.reverses_payment_id, Some(paid.id));
    assert_eq!(
        p.get_payment(&manager, COMPANY, paid.id)
            .await?
            .reversed_by_payment_id,
        Some(reversal.id)
    );
    assert_eq!(
        p.get_document(&manager, COMPANY, b.id).await?.outstanding,
        Some(money("70"))
    );
    refused(
        "double reversal",
        p.reverse_payment(&manager, COMPANY, paid.id, date(2026, 12, 21), None)
            .await,
        |e| matches!(e, PayablesError::AlreadyReversed),
    )?;
    refused(
        "reverse a reversal",
        p.reverse_payment(&manager, COMPANY, reversal.id, date(2026, 12, 21), None)
            .await,
        |e| matches!(e, PayablesError::ReversalNotReversible),
    )?;
    refused(
        "backdated payment cannot overdraw future balance",
        p.submit_payment(
            &manager,
            COMPANY,
            &request(b.id, bank.id, money("40"), date(2026, 12, 13)),
        )
        .await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    let mut note = bill_input("NP3-NOTE", expense.id, money("40"));
    note.document_date = date(2026, 12, 13);
    note.due_date = None;
    refused(
        "backdated note cannot overdraw future balance",
        p.create_debit_note(&manager, COMPANY, b.id, &note).await,
        |e| matches!(e, PayablesError::ExceedsOutstanding),
    )?;
    let before = p
        .aged_payables(
            &manager,
            COMPANY,
            date(2026, 12, 16),
            &AgingQuery::default(),
        )
        .await?;
    assert_eq!(
        before
            .parties
            .iter()
            .find(|p| p.party_key == "np3-party")
            .ok_or(Error::NotFound)?
            .total,
        money("30")
    );
    // Pending requests reserve nothing. Concurrent decisions share the company lock.
    let candidate = request(b.id, bank.id, money("60"), date(2026, 12, 21));
    let a = p.submit_payment(&manager, COMPANY, &candidate).await?;
    let z = p.submit_payment(&manager, COMPANY, &candidate).await?;
    let (a_result, z_result) = tokio::join!(
        p.approve_payment(&reviewer, COMPANY, a.id),
        p.approve_payment(&reviewer, COMPANY, z.id)
    );
    let loser = match (a_result, z_result) {
        (Ok(_), Err(PayablesError::Invalid(_))) => z.id,
        (Err(PayablesError::Invalid(_)), Ok(_)) => a.id,
        other => return Err(format!("concurrent payment approvals: {other:?}").into()),
    };
    assert_eq!(
        p.get_payment_request(&manager, COMPANY, loser)
            .await?
            .status,
        "submitted"
    );
    refused(
        "reject reason required",
        p.reject_payment(&reviewer, COMPANY, loser, " ").await,
        |e| matches!(e, PayablesError::Invalid(_)),
    )?;
    let rejected = p
        .reject_payment(&reviewer, COMPANY, loser, "Another request paid it")
        .await?;
    assert_eq!(rejected.status, "rejected");
    // A bill keeps its original control account after configuration and activation change.
    p.put_settings(
        &manager,
        COMPANY,
        &SettingsInput {
            settings: Settings {
                payable_account_id: other_payable.id,
                quantity_tolerance: None,
                price_tolerance: None,
            },
            reason: Some("New payment control".into()),
        },
    )
    .await?;
    let b2 = post_bill(
        &p,
        &manager,
        &reviewer,
        &bill_input("NP3-NEXT-BILL", expense.id, money("100")),
    )
    .await?;
    // The ledger requires zero balance before deactivation. Show the external
    // adjustment explicitly in tie-out rather than bypassing that control.
    ledger
        .post_entry(
            &manager,
            COMPANY,
            &NewEntry {
                entry_date: date(2026, 12, 21),
                description: "Zero control before deactivation proof".into(),
                reference: None,
                lines: vec![
                    NewLine::debit(payable.id, money("10")),
                    NewLine::credit(bank.id, money("10")),
                ],
            },
            &PostOptions::default(),
        )
        .await?;
    ledger
        .update_account(
            &manager,
            COMPANY,
            payable.id,
            &AccountChange {
                active: Some(false),
                ..Default::default()
            },
        )
        .await?;
    let mut combined = request(b.id, bank.id, money("10"), date(2026, 12, 22));
    combined.allocations.push(AllocationInput {
        document_id: b2.id,
        amount: money("20"),
    });
    let combined = p.submit_payment(&manager, COMPANY, &combined).await?;
    let combined = p.approve_payment(&reviewer, COMPANY, combined.id).await?;
    let entry = ledger
        .get_entry(&manager, COMPANY, combined.entry_id)
        .await?;
    assert_eq!(entry.lines.len(), 3);
    assert!(entry
        .lines
        .iter()
        .any(|l| l.account_id == payable.id && l.debit == money("10")));
    assert!(entry
        .lines
        .iter()
        .any(|l| l.account_id == other_payable.id && l.debit == money("20")));
    // Tax snapshots are grouped by type and pinned, even after new postings.
    let tax = p
        .create_tax_code(
            &manager,
            COMPANY,
            &NewTaxCode {
                code: "NP3-TAX".into(),
                details: TaxCodeDetails {
                    name: "Payment proof tax".into(),
                    rate: "5".into(),
                    account_id: expense.id,
                    active: true,
                    reason: None,
                },
            },
        )
        .await?;
    let mut taxable = bill_input("NP3-TAX-BILL", expense.id, money("100"));
    taxable.lines[0].tax_code_id = Some(tax.id);
    let taxable = post_bill(&p, &manager, &reviewer, &taxable).await?;
    let taxes = p
        .tax_totals(
            &manager,
            COMPANY,
            &TaxQuery {
                from: Some(date(2026, 12, 1)),
                to: Some(date(2026, 12, 31)),
                through_entry_number: taxable.entry_number,
            },
        )
        .await?;
    let own = taxes
        .iter()
        .find(|t| t.tax_code_id == tax.id)
        .ok_or(Error::NotFound)?;
    assert_eq!(own.taxable_amount, money("100"));
    assert_eq!(own.tax_amount, money("5"));
    let tied = p
        .tie_out_control_accounts(&manager, COMPANY, date(2026, 12, 31), None)
        .await?;
    let our = tied
        .accounts
        .iter()
        .find(|a| a.account_id == other_payable.id)
        .ok_or(Error::NotFound)?;
    assert_eq!(our.difference, Money::ZERO);
    assert_eq!(our.subledger_balance, money("185"));
    ledger
        .post_entry(
            &manager,
            COMPANY,
            &NewEntry {
                entry_date: date(2026, 12, 23),
                description: "Unowned control difference proof".into(),
                reference: None,
                lines: vec![
                    NewLine::debit(expense.id, money("3")),
                    NewLine::credit(other_payable.id, money("3")),
                ],
            },
            &PostOptions::default(),
        )
        .await?;
    let changed = p
        .tie_out_control_accounts(&manager, COMPANY, date(2026, 12, 31), None)
        .await?;
    let our = changed
        .accounts
        .iter()
        .find(|a| a.account_id == other_payable.id)
        .ok_or(Error::NotFound)?;
    assert_eq!(our.difference, money("-3"));
    assert!(our
        .other_postings
        .iter()
        .any(|p| p.owner_module.is_none() && p.balance == money("3")));
    let reproduced = p
        .tie_out_control_accounts(
            &manager,
            COMPANY,
            date(2026, 12, 31),
            Some(tied.through_entry_number),
        )
        .await?;
    assert_eq!(reproduced.total_difference, tied.total_difference);
    let page = p
        .list_payments(
            &manager,
            COMPANY,
            &PaymentQuery {
                document_id: Some(b.id),
                limit: Some(1),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(page.len(), 1);
    let next = p
        .list_payments(
            &manager,
            COMPANY,
            &PaymentQuery {
                document_id: Some(b.id),
                after: Some((page[0].payment_date, page[0].id)),
                ..Default::default()
            },
        )
        .await?;
    assert!(!next.is_empty());
    assert!(next.iter().any(|p| p.id == reversal.id));
    let requests = p
        .list_payment_requests(
            &manager,
            COMPANY,
            &PaymentRequestQuery {
                status: Some("rejected".into()),
                ..Default::default()
            },
        )
        .await?;
    assert!(requests.iter().any(|r| r.id == loser));
    let first_page = p
        .list_payment_requests(
            &manager,
            COMPANY,
            &PaymentRequestQuery {
                status: Some("rejected".into()),
                after: Some(0),
                limit: Some(1),
            },
        )
        .await?;
    assert_eq!(first_page.len(), 1);
    let cursor = first_page[0].request_number;
    let next_page = p
        .list_payment_requests(
            &manager,
            COMPANY,
            &PaymentRequestQuery {
                status: Some("rejected".into()),
                after: Some(cursor),
                limit: Some(1),
            },
        )
        .await?;
    assert_eq!(next_page.len(), 1);
    assert!(next_page[0].request_number > cursor);

    for dataset in p.evidence_datasets() {
        let page = p
            .evidence_page(&manager, COMPANY, dataset, Some(1), None)
            .await?;
        assert!(!page.fields.is_empty());
        if let Some(cursor) = page.next {
            let next = p
                .evidence_page(&manager, COMPANY, dataset, Some(500), Some(cursor))
                .await?;
            assert!(next
                .rows
                .iter()
                .all(|r| r["id"].as_i64().is_some_and(|id| id > cursor)));
        }
    }
    refused(
        "foreign evidence",
        p.evidence_page(&foreign, COMPANY, "payment", None, None)
            .await,
        |e| {
            matches!(
                e,
                PayablesError::Unauthorized | PayablesError::CompanyNotFound
            )
        },
    )?;
    refused(
        "evidence invalid bound",
        p.evidence_page(&manager, COMPANY, "payment", Some(501), None)
            .await,
        |e| matches!(e, PayablesError::InvalidOption),
    )?;
    refused(
        "aging invalid boundaries",
        p.aged_payables(
            &manager,
            COMPANY,
            date(2026, 12, 31),
            &AgingQuery {
                bucket_days: vec![60, 30],
                through_entry_number: None,
            },
        )
        .await,
        |e| matches!(e, PayablesError::InvalidOption),
    )?;
    let c = db.get().await?;
    raw_refusal(
        &c,
        "immutable payment",
        &format!(
            "UPDATE finance_payables_payments SET reference='changed' WHERE id={}",
            paid.id
        ),
    )
    .await?;
    raw_refusal(
        &c,
        "immutable request",
        &format!(
            "DELETE FROM finance_payables_payment_requests WHERE id={}",
            r.id
        ),
    )
    .await?;
    raw_refusal(
        &c,
        "immutable decision",
        &format!(
            "DELETE FROM finance_payables_payment_request_events WHERE request_id={}",
            r.id
        ),
    )
    .await?;
    raw_refusal(
        &c,
        "no request truncate",
        "TRUNCATE finance_payables_payment_request_events",
    )
    .await?;
    raw_refusal(&c,"late payment allocation",&format!("INSERT INTO finance_payables_payment_allocations(tenant_id,company_id,payment_id,line_number,document_id,amount) VALUES(1,1,{},99,{},1)",paid.id,b2.id)).await?;
    raw_refusal(&c,"late request allocation",&format!("INSERT INTO finance_payables_payment_request_allocations(tenant_id,company_id,request_id,line_number,document_id,amount) VALUES(1,1,{},99,{},1)",r.id,b2.id)).await?;
    raw_refusal(&c,"request amount differs from allocations",&format!("BEGIN; INSERT INTO finance_payables_payment_requests(tenant_id,company_id,payment_date,party_key,party_name,paid_from_account_id,amount,currency_code,submitted_by_user_id) VALUES(1,1,'2026-12-22','np3-party','Proof',{},10,'MYR',1); COMMIT",bank.id)).await?;
    println!("NP3 complete: requests, independent approvals, concurrent balance refusal, reversals, temporal minima, inactive settlements, pinned aging/tax/tie-out, evidence, attachments and database guards");
    Ok(())
}
