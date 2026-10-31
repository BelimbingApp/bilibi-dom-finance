//! Disposable multi-control browser/MCP proof on a launcher-seeded database.
//! Creates business data only through owner facades, using the launcher's admin.
use bilibi_core::agent_access::{
    code_challenge, AgentAccess, AuthorizeParams, ClientRegistration, CodeExchange,
};
use bilibi_database::{pool, Error};
use bilibi_finance_ledger::{
    AccountType, Ledger, NewAccount, NewBook, NewFiscalYear, OpeningBalances,
};
use bilibi_finance_payables::{Payables, Settings, SettingsInput, VendorInput};
use bilibi_finance_receivables::Receivables;
use bilibi_finance_walkthrough_fixture::sign_in;
use bilibi_workflow::{ApprovalPolicyChange, Engine};
use jiff::civil::date;
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    time::{Duration, SystemTime},
};

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
#[tokio::main]
async fn main() -> Outcome<()> {
    if std::env::var("BILIBI_AGENT_PROOF").as_deref() != Ok("disposable") {
        return Err(Error::Invalid.into());
    }
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let origin = std::env::var("APP_ORIGIN")?;
    let app = bilibi_web::App::new(db.clone(), &origin)?;
    bilibi_finance_walkthrough_fixture::without_step_up(&app.services)?;
    let scope = sign_in(&db, &app.services, 1, 1).await?;
    let ledger = app.services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let ar = app
        .services
        .get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ap = app
        .services
        .get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let company = 1;
    if ledger.get_book(&scope, company).await.is_ok() {
        return Err("fixture requires an empty book".into());
    }
    let currencies = app
        .services
        .get::<bilibi_currency::Currencies>(bilibi_currency::SERVICE)?;
    let boundary = app
        .services
        .get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    let mut client = db.get().await?;
    let mut tx = boundary
        .begin(
            &mut client,
            scope.mutation_context("finance.walkthrough.currency"),
        )
        .await?;
    currencies
        .set_enabled(&mut tx, &scope, "MYR", true)
        .await?
        .map_err(|e| format!("currency: {e:?}"))?;
    tx.commit().await?;
    drop(client);
    ledger
        .open_book(
            &scope,
            company,
            &NewBook {
                currency_code: "MYR".into(),
                minor_units: 2,
            },
        )
        .await?;
    for (code, name, kind) in [
        ("1000", "Receipt account", AccountType::Asset),
        ("1100", "Receivables", AccountType::Asset),
        ("1110", "Historical receivables", AccountType::Asset),
        ("2000", "Payables", AccountType::Liability),
        ("2010", "Historical payables", AccountType::Liability),
        ("3000", "Capital", AccountType::Equity),
        ("4000", "Income", AccountType::Income),
        ("5000", "Expense", AccountType::Expense),
    ] {
        ledger
            .create_account(&scope, company, &NewAccount::new(code, name, kind))
            .await?;
    }
    let chart = ledger.list_accounts(&scope, company).await?;
    let account = |code: &str| {
        chart
            .iter()
            .find(|a| a.code == code)
            .expect("fixture account")
            .id
    };
    ledger
        .open_fiscal_year(
            &scope,
            company,
            &NewFiscalYear {
                start_date: Some(date(2026, 1, 1)),
                end_date: None,
            },
        )
        .await?;
    ledger
        .set_duty_separation(
            &scope,
            company,
            false,
            Some("Disposable single-person browser proof"),
        )
        .await?;
    let engine = app.services.get::<Engine>(bilibi_workflow::SERVICE)?;
    let mut policy = Ledger::journal_approval_policy();
    policy.allow_preparer = true;
    let mut client = db.get().await?;
    let mut tx = ledger
        .write(
            &mut client,
            &scope,
            company,
            &[bilibi_finance_ledger::MANAGE_DUTY_SEPARATION],
            "finance.walkthrough.launcher_rules",
        )
        .await?;
    engine
        .publish_approval_policy_in(
            &mut tx,
            &scope,
            ApprovalPolicyChange {
                case: "finance.ledger.manual_journal",
                company,
                expected: None,
                policy,
                reason: "Disposable single-person browser proof",
            },
        )
        .await?;
    tx.commit().await?;
    drop(client);
    ar.configure(
        &scope,
        company,
        &json!({"receivable_account_id":account("1100"),"income_account_id":account("4000")}),
    )
    .await?;
    let series = ar.create_series(&scope, company, &json!({"code":"OPENING","name":"Opening invoices","kind":"invoice","prefix":"OPEN-","number_width":4})).await?;
    let customer = ar.create_customer(&scope, company, &json!({"new_company":{"name":"Conversion proof counterparty"},"payment_terms_days":30})).await?;
    ap.put_settings(
        &scope,
        company,
        &SettingsInput {
            settings: Settings {
                payable_account_id: account("2000"),
                quantity_tolerance: None,
                price_tolerance: None,
            },
            reason: Some("Disposable browser proof".into()),
        },
    )
    .await?;
    let vendor = ap
        .create_vendor(
            &scope,
            company,
            &VendorInput {
                record_kind: Some(customer.party.kind.clone()),
                record_id: Some(customer.party.id),
                new_company: None,
                new_person: None,
                payment_terms_days: 30,
                payable_account_id: None,
                expense_account_id: None,
                tax_code_id: None,
                active: true,
            },
        )
        .await?;
    let source = "source_id,account_code,debit,credit\nbank,1000,10000,\nreceivables,1100,2500,\nhistorical-receivables,1110,500,\nexpense,5000,2000,\npayables,2000,,1200\nhistorical-payables,2010,,300\ncapital,3000,,8000\nincome,4000,,5500\n";
    let journal = ledger
        .submit_opening_balances(
            &scope,
            company,
            &OpeningBalances {
                conversion_date: date(2026, 7, 1),
                currency_code: "MYR".into(),
                source_reference: "MULTI-CONTROL-BALANCE".into(),
                csv: source.into(),
                replaces_journal_id: None,
            },
        )
        .await?;
    ledger.approve_journal(&scope, company, journal.id).await?;
    let ar_csv = format!("source_id,customer_code,document_reference,document_date,due_date,original_total,outstanding,control_account_code,party_kind\ninvoice-17,{},OLD-INV-17,2026-05-15,2026-06-30,5000,2500,1100,company\ninvoice-18,{},OLD-INV-18,2026-04-15,2026-05-31,1000,500,1110,company\n",customer.code,customer.code);
    let ap_csv = format!("source_id,vendor_code,vendor_name,document_reference,document_date,due_date,original_total,outstanding,control_account_code,party_kind\nbill-29,{},Conversion proof counterparty,OLD-BILL-29,2026-05-01,2026-06-15,2500,1200,2000,company\nbill-30,{},Conversion proof counterparty,OLD-BILL-30,2026-04-01,2026-05-15,900,300,2010,company\n",vendor.code,vendor.code);
    let classifications = [
        ("1000", "not_a_control"),
        ("1100", "receivables_control"),
        ("1110", "receivables_control"),
        ("2000", "payables_control"),
        ("2010", "payables_control"),
    ]
    .into_iter()
    .map(|(code, classification)| (account(code).to_string(), json!(classification)))
    .collect::<serde_json::Map<_, _>>();
    let path = std::env::var("CONVERSION_FIXTURE_DATA")?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    write!(
        file,
        "{}",
        json!({"company_id":company,"series_id":series.id,"ar_csv":ar_csv,"ap_csv":ap_csv,"opening_account_classifications":classifications})
    )?;
    let access = app
        .services
        .get::<AgentAccess>("core/agent_access.grants")?;
    let callback = "https://example.test/conversion-proof/callback";
    let registration = access.register_client(&serde_json::from_value::<ClientRegistration>(json!({"client_name":"Conversion walkthrough","redirect_uris":[callback],"token_endpoint_auth_method":"none"}))?).await?;
    let resource = format!("{}/mcp", origin.trim_end_matches('/'));
    let verifier = "conversion-walkthrough-verifier-with-at-least-forty-three-characters";
    let params = AuthorizeParams {
        client_id: registration["client_id"]
            .as_str()
            .ok_or("client ID")?
            .into(),
        redirect_uri: callback.into(),
        response_type: "code".into(),
        resource: resource.clone(),
        scope: Some("bilibi".into()),
        code_challenge: code_challenge(verifier),
        code_challenge_method: "S256".into(),
        state: Some("conversion-walkthrough".into()),
    };
    let expiry = SystemTime::now() + Duration::from_secs(3600);
    let code = access.authorize(&scope, &params, &resource, expiry).await?;
    let token = access
        .exchange_code(
            &CodeExchange {
                code,
                client_id: params.client_id,
                redirect_uri: params.redirect_uri,
                code_verifier: verifier.into(),
                resource: resource.clone(),
            },
            &resource,
        )
        .await?;
    let mut credentials = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(std::env::var("BILIBI_AGENT_CREDENTIALS")?)?;
    write!(
        credentials,
        "{}",
        json!({"approver":{"mcp":token.access_token}})
    )?;
    println!("Disposable launcher conversion fixture and private MCP credential ready.");
    Ok(())
}
