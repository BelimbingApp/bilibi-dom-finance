//! Running NR1 proof through the real Receivables facade and PostgreSQL,
//! mirroring Bilimbi Receivables setup/draft scenarios. Disposable data only.
use crate::sign_in;
use bilibi_database::{Db, Error};
use bilibi_finance_ledger::{AccountChange, AccountType, Ledger, Money, NewAccount};
use bilibi_finance_receivables::{DocumentQuery, Page, ReceivableError as R, Receivables};
use bilibi_module_registry::Services;
use serde_json::{json, Value};
use std::fmt::Debug;

#[macro_export]
macro_rules! walkthrough_receivables_roles {
    () => {
        $crate::inventory::submit! { $crate::registry::Contribution::new($crate::authz::VOCABULARY, "walkthrough.fixture.finance.receivables", $crate::registry::Provenance { module: "finance/receivables", source: file!(), line: line!() }, &$crate::authz::VocabularyContribution {
            domains: &[], verbs: &[], capabilities: &[],
            roles: &[
                $crate::authz::RoleDefinition { code: "manager", name: "manager", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.receivables.view", "finance.receivables.setup.manage", "finance.receivables.customers.manage", "finance.receivables.documents.manage",
                ] },
                $crate::authz::RoleDefinition { code: "viewer", name: "viewer", description: "Walkthrough fixture role", grant_all: false, capabilities: &["finance.receivables.view"] },
            ], system_principals: &[],
        }) }
    };
}

fn field<T: Debug>(result: Result<T, R>, name: &str, message: &str) {
    match result {
        Err(R::Invalid(errors)) => {
            assert!(
                errors
                    .iter()
                    .any(|e| e.field == name && e.message == message),
                "{errors:?}"
            );
            println!("refused: {name} {message}");
        }
        other => panic!("expected field refusal for {name}: {other:?}"),
    }
}

pub async fn run(db: &Db, services: &Services) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n== Receivables NR1: setup and drafts");
    let ar = services.get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let manager = sign_in(db, services, 1, 1).await?;
    let viewer = sign_in(db, services, 1, 2).await?;
    let admin = sign_in(db, services, 1, 4).await?;
    let other = sign_in(db, services, 2, 5).await?;
    let client = db.get().await?;
    let chart = ledger.list_accounts(&manager, 1).await?;
    let account = |code: &str| {
        chart
            .iter()
            .find(|a| a.code == code)
            .map(|a| a.id)
            .ok_or(Error::NotFound)
    };
    let control = account("1100")?;
    let income = account("4000")?;
    let liability = account("2100")?;
    let units = ledger.get_book(&manager, 1).await?.minor_units;
    let money = |s| Money::parse(s, units).expect("fixture money");
    assert!(matches!(
        ar.get_setup(&manager, 1).await,
        Err(R::NotConfigured)
    ));
    assert!(matches!(
        ar.configure(&viewer, 1, &json!({})).await,
        Err(R::Unauthorized)
    ));
    assert!(matches!(
        ar.get_setup(&other, 1).await,
        Err(R::CompanyNotFound) | Err(R::Unauthorized)
    ));
    field(
        ar.configure(
            &manager,
            1,
            &json!({"receivable_account_id":income,"income_account_id":income}),
        )
        .await,
        "receivable_account_id",
        "must be a ledger asset account",
    );
    let setup = ar
        .configure(
            &manager,
            1,
            &json!({"receivable_account_id":control,"income_account_id":income}),
        )
        .await?;
    assert_eq!(setup.income_account_id, income);
    ar.configure(&manager, 1, &json!({})).await?;
    let history = ar.list_setup_changes(&manager, 1, &Page::default()).await?;
    assert_eq!(history.len(), 2);
    assert!(history
        .iter()
        .all(|h| h.old_value.is_none() && h.changed_by_user_id == 1 && !h.changed_at.is_empty()));
    let next = ar
        .list_setup_changes(
            &manager,
            1,
            &Page {
                limit: Some(1),
                after: Some(history[0].id),
            },
        )
        .await?;
    assert_eq!(next[0].id, history[1].id);
    println!("setup: two initial changes, no history for a no-op, exclusive paging");

    let series = ar.create_series(&manager, 1, &json!({"code":" INV ","name":" Invoices ","kind":"invoice","prefix":"I-","number_width":4})).await?;
    let credits = ar
        .create_series(
            &manager,
            1,
            &json!({"code":"CR","name":"Credit notes","kind":"credit_note","prefix":"C-"}),
        )
        .await?;
    let payment = ar
        .create_series(
            &manager,
            1,
            &json!({"code":"PAY","name":"Payments","kind":"payment","prefix":"P-"}),
        )
        .await?;
    field(
        ar.create_series(
            &manager,
            1,
            &json!({"code":"DUP","name":"Duplicate prefix","kind":"invoice","prefix":"I-"}),
        )
        .await,
        "prefix",
        "has already been taken",
    );
    let unchanged = ar
        .update_series(
            &manager,
            1,
            series.id,
            &json!({"code":{},"kind":false,"prefix":null,"number_width":99,"name":"Renamed"}),
        )
        .await?;
    assert_eq!(
        (
            unchanged.code.as_str(),
            unchanged.prefix.as_str(),
            unchanged.number_width
        ),
        ("INV", "I-", 4)
    );
    assert_eq!(ar.list_series(&manager, 1).await?.len(), 3);
    let tax = ar
        .create_tax_code(
            &manager,
            1,
            &json!({"code":"T10","name":"Tax 10","rate":"10.00000","account_id":liability}),
        )
        .await?;
    field(
        ar.create_tax_code(
            &manager,
            1,
            &json!({"code":"FINE","name":"Fine","rate":"0.00001","account_id":liability}),
        )
        .await,
        "rate",
        "has more than 4 decimal places",
    );
    let unchanged = ar
        .update_tax_code(
            &manager,
            1,
            tax.id,
            &json!({"code":null,"rate":{},"name":"Renamed tax"}),
        )
        .await?;
    assert_eq!(unchanged.rate.normalize().to_string(), "10");
    let customer = ar.create_customer(&manager, 1, &json!({"code":" A ","name":" Customer A ","country_iso":" my ","payment_terms_days":"30","email":"unvalidated","line1":" Street "})).await?;
    assert_eq!(customer.country_iso.as_deref(), Some("MY"));
    assert_eq!(customer.line1.as_deref(), Some("Street"));
    let unchanged = ar
        .update_customer(
            &manager,
            1,
            customer.id,
            &json!({"code":{},"name":"New name"}),
        )
        .await?;
    assert_eq!(unchanged.code, "A");
    field(
        ar.create_customer(&manager, 1, &json!({"code":"A","name":"Duplicate"}))
            .await,
        "code",
        "has already been taken",
    );
    field(
        ar.create_customer(
            &manager,
            1,
            &json!({"code":"BAD","name":"Bad","country_iso":"M1"}),
        )
        .await,
        "country_iso",
        "has invalid format",
    );
    assert!(matches!(
        ar.get_customer(&viewer, 1, customer.id).await,
        Err(R::Unauthorized)
    ));
    assert_eq!(
        ar.get_customer(&manager, 1, customer.id).await?.name,
        "New name"
    );
    assert_eq!(ar.list_customers(&manager, 1).await?.len(), 1);
    assert_eq!(ar.list_tax_codes(&manager, 1).await?.len(), 1);
    println!(
        "code lists: uniqueness, immutable identities, tax precision and flat customer addresses"
    );

    let header = json!({"kind":"invoice","series_id":series.id,"customer_id":customer.id});
    let empty = ar.create_draft(&manager, 1, &header).await?;
    assert!(
        empty.lines.is_empty()
            && empty.total.is_zero()
            && empty.issue_date.is_none()
            && empty.document_number.is_none()
    );
    let line = json!({"description":" Half up ","quantity":"1.00000","unit_price":"0.045","tax_code_id":tax.id,"amount":"999","tax_amount":"999"});
    let with_lines = |lines: Value| {
        let mut v = header.clone();
        v["lines"] = lines;
        v
    };
    let draft = ar
        .create_draft(
            &manager,
            1,
            &with_lines(json!([line.clone(), line.clone()])),
        )
        .await?;
    assert_eq!(draft.subtotal, money("0.10"));
    assert_eq!(draft.tax_total, money("0.02"));
    assert_eq!(draft.total, money("0.12"));
    assert_eq!(draft.tax_lines[0].taxable_amount, money("0.10"));
    assert!(draft.lines.iter().all(|l| l.amount == money("0.05")
        && l.tax_amount == money("0.01")
        && l.account_id == income));
    assert!(
        draft.document_number.is_none()
            && draft.document_reference.is_none()
            && draft.outstanding.is_none()
    );
    println!("rounding: 2 × (1 × 0.045 at 10%) = subtotal 0.10, tax 0.02, total 0.12");
    for (lines, field_name, message) in [
        (
            json!([{"description":"Overflow","quantity":"9999999999999999","unit_price":"9999999999999999"}]),
            "amount",
            "must be less than 1E+16",
        ),
        (
            json!([{"description":"Rounded limit","quantity":"1","unit_price":"9999999999999999.9999"}]),
            "amount",
            "must be less than 1E+16",
        ),
        (
            json!([{"description":"A","quantity":"1","unit_price":"6000000000000000"},{"description":"B","quantity":"1","unit_price":"4000000000000000"}]),
            "total",
            "must be less than 1E+16",
        ),
        (
            json!([{"description":"Fine","quantity":"0.00001","unit_price":"1"}]),
            "quantity",
            "has more than 4 decimal places",
        ),
        (
            json!([{"description":"Wrong account","quantity":"1","unit_price":"1","account_id":control}]),
            "account_id",
            "must be a ledger income account",
        ),
    ] {
        field(
            ar.update_draft(
                &manager,
                1,
                draft.id,
                &json!({"description":"must roll back","lines":lines}),
            )
            .await,
            field_name,
            message,
        );
    }
    assert_eq!(ar.get_document(&manager, 1, draft.id).await?, draft);
    field(
        ar.create_draft(
            &manager,
            1,
            &json!({"kind":"invoice","series_id":payment.id,"customer_id":customer.id}),
        )
        .await,
        "series_id",
        "does not number invoice documents",
    );
    field(ar.create_draft(&manager, 1, &json!({"kind":"credit_note","series_id":credits.id,"customer_id":customer.id,"credited_document_id":empty.id})).await, "credited_document_id", "is not an issued invoice of this customer");
    field(
        ar.update_draft(
            &manager,
            1,
            draft.id,
            &json!({"issue_date":"2026-10-10","due_date":"2026-10-09"}),
        )
        .await,
        "due_date",
        "cannot be before the issue date",
    );

    let next_income = ledger
        .create_account(
            &manager,
            1,
            &NewAccount::new("NR1-income", "Another income account", AccountType::Income),
        )
        .await?;
    ar.configure(&manager, 1, &json!({"income_account_id":next_income.id}))
        .await?;
    let old_ids: Value = client.query_one("SELECT jsonb_agg(id ORDER BY id) FROM finance_receivables_document_lines WHERE tenant_id=1 AND company_id=1 AND document_id=$1", &[&draft.id]).await?.get(0);
    let kept = ar
        .update_draft(
            &manager,
            1,
            draft.id,
            &json!({"lines":null,"kind":"credit_note"}),
        )
        .await?;
    assert_eq!(kept, draft);
    let new_ids: Value = client.query_one("SELECT jsonb_agg(id ORDER BY id) FROM finance_receivables_document_lines WHERE tenant_id=1 AND company_id=1 AND document_id=$1", &[&draft.id]).await?.get(0);
    assert_eq!(old_ids, new_ids);
    let replaced = ar
        .update_draft(&manager, 1, draft.id, &json!({"lines":line}))
        .await?;
    assert_eq!(replaced.lines.len(), 1);
    assert_eq!(replaced.lines[0].account_id, next_income.id);
    let cleared = ar
        .update_draft(&manager, 1, empty.id, &json!({"lines":[]}))
        .await?;
    assert!(cleared.lines.is_empty() && cleared.tax_lines.is_empty() && cleared.total.is_zero());
    // Deactivation does not prevent renaming a tax code with its unchanged account.
    ledger
        .update_account(
            &manager,
            1,
            liability,
            &AccountChange {
                active: Some(false),
                ..Default::default()
            },
        )
        .await?;
    ar.update_tax_code(
        &manager,
        1,
        tax.id,
        &json!({"name":"Inactive account kept"}),
    )
    .await?;
    ledger
        .update_account(
            &manager,
            1,
            liability,
            &AccountChange {
                active: Some(true),
                ..Default::default()
            },
        )
        .await?;
    ar.update_tax_code(&manager, 1, tax.id, &json!({"active":false}))
        .await?;
    field(ar.update_draft(&manager, 1, draft.id, &json!({"lines":[{"description":"Inactive tax","quantity":"1","unit_price":"1","tax_code_id":tax.id}]})).await, "tax_code_id", "is an inactive tax code");
    ar.update_tax_code(&manager, 1, tax.id, &json!({"active":true}))
        .await?;
    ar.update_customer(&manager, 1, customer.id, &json!({"active":false}))
        .await?;
    field(
        ar.update_draft(&manager, 1, draft.id, &json!({})).await,
        "customer_id",
        "is an inactive customer",
    );
    ar.update_customer(&manager, 1, customer.id, &json!({"active":true}))
        .await?;
    let discarded = ar.discard_draft(&manager, 1, draft.id).await?;
    assert_eq!(discarded.lines, replaced.lines);
    assert!(discarded.discarded_at.is_some() && discarded.document_number.is_none());
    assert!(matches!(
        ar.update_draft(&manager, 1, draft.id, &json!({})).await,
        Err(R::NotADraft)
    ));
    assert!(matches!(
        ar.discard_draft(&manager, 1, draft.id).await,
        Err(R::NotADraft)
    ));
    ar.configure(&manager, 1, &json!({"income_account_id":income}))
        .await?;
    println!("drafts: invalid replacement rolled back; null preserves IDs/snapshots; replace, clear and kept discard");

    let query = DocumentQuery {
        page: Page {
            limit: Some(1),
            after: Some(empty.id),
        },
        status: Some("discarded".into()),
        ..Default::default()
    };
    assert_eq!(
        ar.list_documents(&manager, 1, &query).await?[0].id,
        draft.id
    );
    assert!(matches!(
        ar.list_documents(
            &manager,
            1,
            &DocumentQuery {
                page: Page {
                    limit: Some(501),
                    after: None
                },
                ..Default::default()
            }
        )
        .await,
        Err(R::InvalidOption)
    ));
    let race_a = json!({"code":"RACE","name":"Race A"});
    let race_b = json!({"code":"RACE","name":"Race B"});
    let (a, b) = tokio::join!(
        ar.create_customer(&manager, 1, &race_a),
        ar.create_customer(&manager, 1, &race_b)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert!(matches!(a.err().or(b.err()), Some(R::Invalid(_))));
    for sql in [
        "UPDATE finance_receivables_setup_changes SET new_value=1 WHERE tenant_id=1 AND company_id=1",
        "DELETE FROM finance_receivables_documents WHERE tenant_id=1 AND company_id=1",
        "UPDATE finance_receivables_series SET prefix='changed' WHERE tenant_id=1 AND company_id=1",
        "UPDATE finance_receivables_tax_codes SET rate=20 WHERE tenant_id=1 AND company_id=1",
        "UPDATE finance_receivables_customers SET code='changed' WHERE tenant_id=1 AND company_id=1",
        "TRUNCATE finance_receivables_document_lines",
    ] { let error = client.batch_execute(sql).await.expect_err("database must refuse"); println!("database refused: {}", error.as_db_error().map(|e| e.message()).unwrap_or("database error")); }
    let audit: i64 = client.query_one("SELECT count(*) FROM base_audit_mutations WHERE auditable_type LIKE 'finance.receivables.%'", &[]).await?.get(0);
    assert!(audit > 20);
    println!("receivables audit mutations: {audit}; concurrent duplicate serialized to one success and one field refusal");
    // Company 3 is disposable and later walkthroughs already expect it archived.
    client
        .batch_execute("UPDATE companies SET status='archived' WHERE tenant_id=1 AND id=3")
        .await?;
    assert!(matches!(
        ar.create_customer(&admin, 3, &json!({"code":"X","name":"Archived"}))
            .await,
        Err(R::CompanyArchived)
    ));
    println!("NR1 proof passed: setup, code lists, arithmetic, drafts, isolation, paging, concurrent writes, archival, SQL guards and audit");
    Ok(())
}
