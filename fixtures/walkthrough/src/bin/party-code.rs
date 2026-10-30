//! Fixed Core codes across Company/Person customers and suppliers on a fresh DB.
//! PARTY_CODE_BROWSER=1 keeps the proved app available for a browser walkthrough.
use bilibi_database::pool;
use bilibi_finance_payables::{Payables, VendorInput};
use bilibi_finance_receivables::{AgingOptions, Receivables};
use bilibi_finance_walkthrough_fixture::{install, sign_in};
use jiff::civil::date;
use serde_json::json;
bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_attachments_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_receivables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_payables_roles!();
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_authz::VOCABULARY,"walkthrough.fixture.party-code",bilibi_module_registry::Provenance {module:"finance/receivables",source:file!(),line:line!()},&bilibi_authz::VocabularyContribution {domains:&[],verbs:&[],capabilities:&[],system_principals:&[],roles:&[bilibi_authz::RoleDefinition{code:"manager",name:"manager",description:"Walkthrough fixture role",grant_all:false,capabilities:&bilibi_finance_receivables::CAPABILITIES}]}) }
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let origin = std::env::var("APP_ORIGIN").unwrap_or("http://127.0.0.1:45479".into());
    bilibi_database::migrate(&db, &bilibi_module_registry::discover()?).await?;
    let app = bilibi_web::App::new(db.clone(), &origin)?;
    bilibi_finance_walkthrough_fixture::without_step_up(&app.services)?;
    let installed = install(&db, &app.services).await?;
    let scope = sign_in(&db, &app.services, 1, 1).await?;
    let ar = app
        .services
        .get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ap = app
        .services
        .get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let account = |code: &str| {
        installed
            .accounts
            .iter()
            .find(|a| a.code == code)
            .unwrap()
            .id
    };
    ar.configure(
        &scope,
        1,
        &json!({"receivable_account_id":account("1100"),"income_account_id":account("4000")}),
    )
    .await?;
    let series = ar.create_series(&scope,1,&json!({"code":"TV","name":"Retail invoices","kind":"invoice","prefix":"TV-","number_width":4})).await?;
    let company = ar
        .create_customer(&scope, 1, &json!({"record_kind":"company","record_id":10}))
        .await?;
    let consumer = ar.create_customer(&scope,1,&json!({"new_person":{"name":"Jamie TV Consumer","kind":"human","email":"jamie@example.test"},"payment_terms_days":30})).await?;
    assert_eq!(company.code, company.party.code);
    assert_eq!(consumer.code, consumer.party.code);
    assert_eq!(consumer.party.kind, "person");
    // A taken Finance reference refuses the write. Codes are never Finance input.
    assert!(ar
        .create_customer(
            &scope,
            1,
            &json!({"record_kind":"person","record_id":consumer.party.id})
        )
        .await
        .is_err());
    assert!(ar
        .update_customer(&scope, 1, consumer.id, &json!({"code":"OTHER"}))
        .await
        .is_err());
    assert!(ar
        .update_customer(
            &scope,
            1,
            consumer.id,
            &json!({"record_kind":"company","record_id":11})
        )
        .await
        .is_err());
    let vendor_input = |kind: &str, id| VendorInput {
        record_kind: Some(kind.into()),
        record_id: Some(id),
        new_company: None,
        new_person: None,
        payment_terms_days: 30,
        payable_account_id: None,
        expense_account_id: None,
        tax_code_id: None,
        active: true,
    };
    let supplier = ap
        .create_vendor(&scope, 1, &vendor_input("company", 11))
        .await?;
    // The same real person may be both a customer and supplier, with one code.
    let person_supplier = ap
        .create_vendor(&scope, 1, &vendor_input("person", consumer.party.id))
        .await?;
    assert_eq!(person_supplier.code, consumer.code);
    assert_eq!(supplier.code, supplier.party.code);
    let draft = ar.create_draft(&scope,1,&json!({"kind":"invoice","series_id":series.id,"customer_id":consumer.id,"issue_date":"2026-10-30","lines":[{"description":"Television","quantity":"1","unit_price":"1200"}]})).await?;
    let issued = ar.issue_document(&scope, 1, draft.id).await?;
    assert_eq!(issued.party_code.as_deref(), Some(consumer.code.as_str()));
    assert!(issued.ledger_entry_id.is_some());
    let aging = ar
        .aged_receivables(&scope, 1, date(2026, 10, 30), &AgingOptions::default())
        .await?;
    let aged = aging
        .customers
        .iter()
        .find(|c| c.customer_id == consumer.id)
        .expect("consumer in aging");
    assert_eq!(aged.code, consumer.code);
    println!("PASS: company customer {} code {}; person customer {} code {}; company supplier {} code {}; person supplier shares {}; TV invoice {} posted and aging carries {}",company.id,company.code,consumer.id,consumer.code,supplier.id,supplier.code,person_supplier.code,issued.id,aged.code);
    if std::env::var("PARTY_CODE_BROWSER").as_deref() == Ok("1") {
        let listener = tokio::net::TcpListener::bind(
            std::env::var("BIND_ADDR").unwrap_or("127.0.0.1:45479".into()),
        )
        .await?;
        println!("Browser at {origin}; manager@example.test / bilibi-dev");
        axum::serve(listener, bilibi_web::router(app)).await?;
    }
    Ok(())
}
