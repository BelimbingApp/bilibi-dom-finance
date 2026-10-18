//! NU1: one disposable browser fixture for the Finance click-through sweep.
//! Run with DATABASE_URL, APP_ORIGIN, BIND_ADDR and BILIBI_SETTINGS_KEY.
//! Identity fixture SQL only; every Finance record is created by its facade.
use bilibi_authz::{RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{pool, Error};
use bilibi_finance_attachments::{Attachments, NewFile};
use bilibi_finance_ledger::{
    AccountType, Ledger, Money, NewAccount, NewEntry, NewLine, PostOptions,
};
use bilibi_finance_payables::{
    DocumentLineInput, NewDocument, Party, Payables, Settings, SettingsInput, DOCUMENT_SOURCE,
};
use bilibi_finance_reconciliation::{NewBankAccount, Reconciliation};
use bilibi_finance_walkthrough_fixture::{
    attachments::configure_storage, install, sign_in, COMPANY,
};
use bilibi_module_registry::{Contribution, Provenance};
use jiff::civil::date;
use serde_json::json;

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
inventory::submit! { Contribution::new(VOCABULARY,"walkthrough.fixture.finance.browser",Provenance {module:"finance/ledger",source:file!(),line:line!()},&VocabularyContribution {
    domains:&[],verbs:&[],capabilities:&[],system_principals:&[],roles:&[
        RoleDefinition {code:"manager",name:"manager",description:"Disposable browser recorder",grant_all:true,capabilities:&[]},
        RoleDefinition {code:"viewer",name:"viewer",description:"Disposable browser reader",grant_all:false,capabilities:&[
            "finance.ledger.view", "finance.attachments.view", "finance.payables.view",
            "finance.einvoice.view", "finance.fixed-assets.view", "finance.payroll.view",
            "finance.reconciliation.view", "finance.sst.view",
        ]},
    ],
}) }

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> Outcome<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let origin = std::env::var("APP_ORIGIN")?;
    bilibi_database::migrate(&db, &bilibi_module_registry::discover()?).await?;
    let app = bilibi_web::App::new(db.clone(), &origin)?;
    let services = &app.services;
    let installed = install(&db, services).await?;
    db.get().await?.batch_execute(
        "INSERT INTO persons(id,tenant_id,name,kind,email) VALUES(4,1,'Alex Admin','human','admin@example.test'),(5,1,'Lee Viewer','human','viewer@example.test') ON CONFLICT DO NOTHING;
         INSERT INTO person_affiliations(id,tenant_id,person_id,organisation_id,title,work_email) VALUES(3,1,4,1,'Reviewer','admin@example.test'),(4,1,5,1,'Reader','viewer@example.test') ON CONFLICT DO NOTHING;
         UPDATE users SET person_id=4,affiliation_id=3,organisation_id=1 WHERE tenant_id=1 AND id=4 AND person_id IS NULL;
         UPDATE users SET person_id=5,affiliation_id=4,organisation_id=1 WHERE tenant_id=1 AND id=2 AND person_id IS NULL;
         SELECT setval(pg_get_serial_sequence('persons','id'),(SELECT max(id) FROM persons));
         SELECT setval(pg_get_serial_sequence('person_affiliations','id'),(SELECT max(id) FROM person_affiliations))"
    ).await?;
    let manager = sign_in(&db, services, 1, 1).await?;
    let admin = sign_in(&db, services, 1, 4).await?;
    let account = |code: &str| {
        installed
            .accounts
            .iter()
            .find(|a| a.code == code)
            .map(|a| a.id)
            .ok_or(Error::NotFound)
    };
    let root = std::env::var("FINANCE_UI_STORAGE")?;
    std::fs::create_dir_all(&root)?;
    configure_storage(&db, services, &admin, &root).await?;
    let payables = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    payables
        .put_settings(
            &manager,
            COMPANY,
            &SettingsInput {
                settings: Settings {
                    payable_account_id: account("2000")?,
                    quantity_tolerance: None,
                    price_tolerance: None,
                },
                reason: None,
            },
        )
        .await?;
    for n in 1..=3 {
        let bill = payables
            .create_bill(
                &manager,
                COMPANY,
                &NewDocument {
                    party: Party {
                        key: "browser-supplier".into(),
                        name: "Paper supplier".into(),
                        tax_id: None,
                        registration_id: None,
                        address: Some("Supplier address".into()),
                    },
                    party_document_number: Some(format!("BROWSER-{n}")),
                    document_date: date(2026, 10, 1),
                    due_date: Some(date(2026, 10, 31)),
                    description: Some("Browser walkthrough".into()),
                    lines: vec![DocumentLineInput {
                        description: "Office supplies".into(),
                        account_id: account("5000")?,
                        amount: Money::parse("72.50", installed.book.minor_units)?,
                        tax_code_id: None,
                        external_order_line_id: None,
                        order_line_id: None,
                        quantity: None,
                    }],
                    tax_lines: None,
                },
            )
            .await?;
        if n > 1 {
            payables.approve_document(&admin, COMPANY, bill.id).await?;
        }
        if n == 3 {
            payables.post_document(&manager, COMPANY, bill.id).await?;
        }
        services
            .get::<Attachments>(bilibi_finance_attachments::SERVICE)?
            .attach(
                &manager,
                COMPANY,
                DOCUMENT_SOURCE,
                bill.id,
                &NewFile {
                    file_name: format!("supplier-{n}.txt"),
                    content: b"Source document for browser proof".to_vec(),
                    content_type: Some("text/plain".into()),
                },
            )
            .await?;
        println!("Bill {n}: /finance/payables/bills/{}", bill.id);
    }
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let cost = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("1200", "Equipment cost", AccountType::Asset),
        )
        .await?;
    let accumulated = ledger
        .create_account(
            &manager,
            COMPANY,
            &NewAccount::new("1290", "Accumulated depreciation", AccountType::Asset),
        )
        .await?;
    let entry = ledger
        .post_entry(
            &manager,
            COMPANY,
            &NewEntry {
                entry_date: date(2026, 10, 1),
                description: "Equipment acquisition".into(),
                reference: None,
                lines: vec![
                    NewLine::debit(cost.id, Money::parse("120", installed.book.minor_units)?),
                    NewLine::credit(
                        account("3000")?,
                        Money::parse("120", installed.book.minor_units)?,
                    ),
                ],
            },
            &PostOptions::default(),
        )
        .await?;
    println!(
        "Acquisition entry {}, line 1; cost {}; accumulated {}",
        entry.id, cost.id, accumulated.id
    );
    let bank = services.get::<Reconciliation>(bilibi_finance_reconciliation::SERVICE)?.create_bank_account(&manager,COMPANY,&NewBankAccount {
        name:"Operating account".into(),ledger_account_id:account("1000")?,charges_account_id:Some(account("5000")?),interest_account_id:Some(account("4000")?),match_window_days:None,active:None,reason:None,
        import_mapping:json!({"date":"Date","description":"Details","reference":"Ref","amount":"Amount","balance":"Balance"}),
    }).await?;
    println!("Bank account: /finance/reconciliation/accounts/{}", bank.id);
    // Existing authority stand-in creates documents and serves submit/poll locally.
    let _authority = bilibi_finance_walkthrough_fixture::einvoice_ui::run(&db, services).await?;
    println!("Finance browser fixture: {origin}; manager@example.test, viewer@example.test; walkthrough password");
    axum::serve(
        tokio::net::TcpListener::bind(std::env::var("BIND_ADDR")?).await?,
        bilibi_web::router(app),
    )
    .await?;
    Ok(())
}
