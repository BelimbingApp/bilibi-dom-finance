//! Disposable opening-state conversion: browser first on Company 001, MCP on
//! clean Company 002, then facade controls on Company 003. No product seeds.
#[path = "../conversion_lifecycle.rs"]
mod conversion_lifecycle;
use bilibi_agent_api::{Handler, Registry};
use bilibi_authz::{RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::{pool, Db};
use bilibi_finance_fixed_assets::{Assumptions, FixedAssets, Method, ReportPins, RevisionInput};
use bilibi_finance_ledger::{
    AccountType, Ledger, Money, NewAccount, NewBook, NewFiscalYear, OpeningBalances, Reversal,
};
use bilibi_finance_payables::{
    DocumentLineInput, NewDocument, Payables, PayablesError, Settings, SettingsInput, VendorInput,
};
use bilibi_finance_receivables::{AgingOptions, Receivables};
use bilibi_finance_walkthrough_fixture::{install, sign_in};
use bilibi_module_registry::{Contribution, Provenance, Services};
use jiff::civil::date;
use serde_json::{json, Value};

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_payables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_reconciliation_roles!();
macro_rules! roles {($key:literal,$module:literal,$caps:path)=>{inventory::submit!{Contribution::new(VOCABULARY,$key,Provenance{module:$module,source:file!(),line:line!()},&VocabularyContribution{domains:&[],verbs:&[],capabilities:&[],system_principals:&[],roles:&[RoleDefinition{code:"manager",name:"manager",description:"Disposable conversion fixture",grant_all:false,capabilities:&$caps}]})}};}
roles!(
    "walkthrough.conversion.receivables",
    "finance/receivables",
    bilibi_finance_receivables::CAPABILITIES
);
roles!(
    "walkthrough.conversion.assets",
    "finance/fixed_assets",
    bilibi_finance_fixed_assets::CAPABILITIES
);
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
const SOURCE: &str = "source_id,account_code,debit,credit\nbank,1000,10000,\nreceivables,1100,3000,\nasset-cost,1200,12000,\nytd-cost,5000,2000,\npayables,2000,,1500\naccumulation,1290,,1800\ncapital,3000,,18200\nytd-sales,4000,,5500\n";
const AR: &str = "source_id,customer_code,document_reference,document_date,due_date,original_total,outstanding,control_account_code,party_kind\ninvoice-17,company_010,OLD-INV-17,2026-05-15,2026-06-30,5000,3000,1100,company\n";
const AP: &str = "source_id,vendor_code,vendor_name,document_reference,document_date,due_date,original_total,outstanding,control_account_code,party_kind\nbill-29,company_010,Conversion vendor,OLD-BILL-29,2026-05-01,2026-06-15,2500,1500,2000,company\n";
async fn setup(db: &Db, services: &Services, company: i64) -> Outcome<Value> {
    let admin = sign_in(db, services, 1, 4).await?;
    let manager = sign_in(db, services, 1, 1).await?;
    let l = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let ar = services.get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ap = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let assets = services.get::<FixedAssets>(bilibi_finance_fixed_assets::SERVICE)?;
    if l.get_book(&admin, company).await.is_err() {
        l.open_book(
            &admin,
            company,
            &NewBook {
                currency_code: "MYR".into(),
                minor_units: 2,
            },
        )
        .await?;
    }
    let chart = l.list_accounts(&admin, company).await?;
    for (code, name, kind, retained) in bilibi_finance_walkthrough_fixture::CHART {
        if !chart.iter().any(|a| a.code == code) {
            let mut new = NewAccount::new(code, name, kind);
            new.retained_earnings = retained;
            l.create_account(&admin, company, &new).await?;
        }
    }
    let chart = l.list_accounts(&admin, company).await?;
    for (code, name, kind) in [
        ("1200", "Asset cost", AccountType::Asset),
        ("1290", "Accumulated depreciation", AccountType::Asset),
        ("6100", "Depreciation expense", AccountType::Expense),
        ("4100", "Asset disposal gain", AccountType::Income),
    ] {
        if !chart.iter().any(|a| a.code == code) {
            l.create_account(&admin, company, &NewAccount::new(code, name, kind))
                .await?;
        }
    }
    if l.list_fiscal_years(&admin, company).await?.is_empty() {
        l.open_fiscal_year(
            &admin,
            company,
            &NewFiscalYear {
                start_date: Some(date(2026, 1, 1)),
                end_date: None,
            },
        )
        .await?;
    }
    let chart = l.list_accounts(&admin, company).await?;
    let account = |code: &str| {
        chart
            .iter()
            .find(|a| a.code == code)
            .expect("fixture account")
            .id
    };
    ar.configure(
        &admin,
        company,
        &json!({"receivable_account_id":account("1100"),"income_account_id":account("4000")}),
    )
    .await?;
    let series = ar.list_series(&admin, company).await?;
    let invoice = if let Some(s) = series.iter().find(|s| s.code == "OPENING") {
        s.clone()
    } else {
        ar.create_series(&admin,company,&json!({"code":"OPENING","name":"Opening invoices","kind":"invoice","prefix":"OPEN-","number_width":4})).await?
    };
    let payment = if let Some(s) = series.iter().find(|s| s.code == "RECEIPT") {
        s.clone()
    } else {
        ar.create_series(&admin,company,&json!({"code":"RECEIPT","name":"Receipts","kind":"payment","prefix":"REC-","number_width":4})).await?
    };
    let customers = ar.list_customers(&admin, company).await?;
    let customer = if let Some(c) = customers.iter().find(|c| c.code == "company_010") {
        c.clone()
    } else {
        ar.create_customer(
            &admin,
            company,
            &json!({"record_kind":"company","record_id":10,"payment_terms_days":30}),
        )
        .await?
    };
    ap.put_settings(
        &admin,
        company,
        &SettingsInput {
            settings: Settings {
                payable_account_id: account("2000"),
                quantity_tolerance: None,
                price_tolerance: None,
            },
            reason: Some("Conversion setup".into()),
        },
    )
    .await?;
    let vendors = ap.list_vendors(&admin, company, 0, 500).await?;
    let vendor = if let Some(vendor) = vendors.iter().find(|v| v.code == "company_010") {
        vendor.clone()
    } else {
        ap.create_vendor(
            &admin,
            company,
            &VendorInput {
                record_kind: Some("company".into()),
                record_id: Some(10),
                new_company: None,
                new_person: None,
                payment_terms_days: 30,
                payable_account_id: None,
                expense_account_id: None,
                tax_code_id: None,
                active: true,
            },
        )
        .await?
    };
    let categories = assets.list_categories(&admin, company).await?;
    let revision = if let Some(c) = categories.iter().find(|c| c.name == "Conversion assets") {
        c.revision.id
    } else {
        assets
            .create_category(
                &admin,
                company,
                "Conversion assets",
                &RevisionInput {
                    cost_account_id: account("1200"),
                    accumulated_account_id: account("1290"),
                    expense_account_id: account("6100"),
                    gain_account_id: account("4100"),
                    loss_account_id: account("6100"),
                    assumptions: Assumptions {
                        method: Method::StraightLine,
                        life_months: 120,
                        annual_rate: None,
                    },
                    reason: "Source assumptions; opening import uses remaining life".into(),
                },
            )
            .await?
            .id
    };
    bilibi_finance_walkthrough_fixture::initialize_journal_approvals(
        db, services, &manager, company,
    )
    .await?;
    if l.opening_context_on(&**db.get().await?, &admin, company)
        .await
        .is_err()
    {
        let j = l
            .submit_opening_balances(
                &manager,
                company,
                &OpeningBalances {
                    conversion_date: date(2026, 7, 1),
                    currency_code: "MYR".into(),
                    source_reference: format!("CONVERSION-{company}"),
                    csv: SOURCE.into(),
                    replaces_journal_id: None,
                },
            )
            .await?;
        l.approve_journal(&admin, company, j.id).await?;
    }
    let reconciliation = services.get::<bilibi_finance_reconciliation::Reconciliation>(
        bilibi_finance_reconciliation::SERVICE,
    )?;
    let bank_account = match reconciliation.list_bank_accounts(&admin, company).await?.into_iter().find(|a| a.ledger_account_id == account("1000")) {
        Some(a) => a,
        None => reconciliation.create_bank_account(&admin, company, &bilibi_finance_reconciliation::NewBankAccount {
            name: "Conversion bank".into(), ledger_account_id: account("1000"),
            import_mapping: json!({"date":"Date","description":"Details","reference":"Ref","amount":"Amount","balance":"Balance"}),
            charges_account_id: None, interest_account_id: None, match_window_days: None, active: None, reason: None,
        }).await?,
    };
    let asset_csv = format!(
        "source_id,description,category_revision_id,acquisition_date,in_service_date,cost,accumulated_depreciation,remaining_life_months,residual_value\nasset-3,Source register asset,{revision},2024-06-01,2024-07-01,12000,1800,96,0\n"
    );
    Ok(
        json!({"company_id":company,"vendor_id":vendor.id,"vendor_key":format!("vendor:{}", vendor.id),"customer_id":customer.id,"invoice_series":invoice.id,"payment_series":payment.id,"bank":account("1000"),"bank_account_id":bank_account.id,"ar":AR,"ap":AP,"expense":account("5000"),"assets":asset_csv,"revision":revision}),
    )
}
async fn prove(db: &Db, services: &Services) -> Outcome<()> {
    let company = 3;
    let config = setup(db, services, company).await?;
    let s = sign_in(db, services, 1, 4).await?;
    let manager = sign_in(db, services, 1, 1).await?;
    let viewer = sign_in(db, services, 1, 2).await?;
    let l = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let ar = services.get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ap = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let assets = services.get::<FixedAssets>(bilibi_finance_fixed_assets::SERVICE)?;
    let trial = l.trial_balance(&s, company, date(2026, 7, 1), None).await?;
    assert_eq!(trial.total_debit.shown(trial.minor_units), "27000.00");
    let before_ar = ar
        .aged_receivables(
            &s,
            company,
            date(2026, 7, 1),
            &AgingOptions {
                through_entry_number: Some(trial.through_entry_number),
                ..Default::default()
            },
        )
        .await?;
    let before_ap = ap
        .aged_payables(
            &s,
            company,
            date(2026, 7, 1),
            &bilibi_finance_payables::AgingQuery {
                through_entry_number: Some(trial.through_entry_number),
                ..Default::default()
            },
        )
        .await?;
    let before_ar_tie = ar
        .tie_out_control_accounts(&s, company, date(2026, 7, 1), &Default::default())
        .await?;
    let before_ap_tie = ap
        .tie_out_control_accounts(&s, company, date(2026, 7, 1), &Default::default())
        .await?;
    let before_assets = assets
        .register_report(&s, company, date(2026, 7, 1), &Default::default())
        .await?;
    let ar_input = json!({"source_reference":"AR-SOURCE","currency_code":"MYR","series_id":config["invoice_series"],"csv":AR});
    let ap_input = json!({"source_reference":"AP-SOURCE","currency_code":"MYR","csv":AP});
    let asset_input =
        json!({"source_reference":"ASSET-SOURCE","currency_code":"MYR","csv":config["assets"]});
    for input in [&ar_input, &ap_input, &asset_input] {
        assert!(input["csv"].as_str().is_some());
    }
    let mut wrong = ar_input.clone();
    wrong["csv"] = json!(AR.replace(",3000,1100", ",3001,1100"));
    assert!(ar
        .import_opening_invoices(&s, company, &wrong)
        .await
        .is_err());
    let mut wrong_ap = ap_input.clone();
    wrong_ap["csv"] = json!(AP.replace(",1500,2000", ",1501,2000"));
    assert!(ap
        .import_opening_bills(&s, company, &wrong_ap)
        .await
        .is_err());
    let mut wrong_asset = asset_input.clone();
    wrong_asset["csv"] = json!(config["assets"]
        .as_str()
        .unwrap()
        .replace(",1800,96", ",1801,96"));
    assert!(assets
        .import_opening_register(&s, company, &wrong_asset)
        .await
        .is_err());
    assert!(ar
        .import_opening_invoices(&manager, company, &ar_input)
        .await
        .is_err());
    assert!(ar
        .import_opening_invoices(&viewer, company, &ar_input)
        .await
        .is_err());
    // A blank, unknown or wrong-kind Core code is a refusal of its row, never a
    // match to a customer or supplier whose Core party cannot be read.
    for (csv, words) in [
        (
            AR.replace("invoice-17,company_010", "invoice-17,"),
            "customer_code is required",
        ),
        (
            AR.replace("company_010", "MISSING-CUSTOMER"),
            "customer_code and party_kind",
        ),
        (
            AR.replace(",company\n", ",person\n"),
            "customer_code and party_kind",
        ),
    ] {
        let mut bad = ar_input.clone();
        bad["csv"] = json!(csv);
        let refusal = format!(
            "{:?}",
            ar.import_opening_invoices(&s, company, &bad)
                .await
                .unwrap_err()
        );
        assert!(refusal.contains(words), "{refusal}");
    }
    for csv in [
        AP.replace("bill-29,company_010", "bill-29,"),
        AP.replace(",company\n", ",person\n"),
    ] {
        let mut bad = ap_input.clone();
        bad["csv"] = json!(csv);
        assert!(ap.import_opening_bills(&s, company, &bad).await.is_err());
    }
    println!("PASS: blank, unknown and wrong-kind Core codes refuse opening invoices and bills");
    let a = ar.import_opening_invoices(&s, company, &ar_input).await?;
    assert_eq!(a, ar.import_opening_invoices(&s, company, &ar_input).await?);
    let mut unknown_vendor = ap_input.clone();
    unknown_vendor["csv"] = json!(AP.replace("company_010", "MISSING-VENDOR"));
    assert!(ap
        .import_opening_bills(&s, company, &unknown_vendor)
        .await
        .is_err());
    let vendor = ap
        .get_vendor(&s, company, config["vendor_id"].as_i64().unwrap())
        .await?;
    let mut vendor_input = VendorInput {
        record_kind: Some("company".into()),
        record_id: Some(vendor.party.id),
        new_company: None,
        new_person: None,
        payment_terms_days: vendor.payment_terms_days,
        payable_account_id: vendor.payable_account_id,
        expense_account_id: vendor.expense_account_id,
        tax_code_id: vendor.tax_code_id,
        active: false,
    };
    ap.update_vendor(&s, company, vendor.id, &vendor_input)
        .await?;
    match ap.import_opening_bills(&s, company, &ap_input).await {
        Err(PayablesError::Invalid(fields)) => assert!(fields.iter().all(|f| f.field == "csv")),
        other => panic!("inactive opening vendor was not refused at the CSV field: {other:?}"),
    }
    vendor_input.active = true;
    ap.update_vendor(&s, company, vendor.id, &vendor_input)
        .await?;
    let b = ap.import_opening_bills(&s, company, &ap_input).await?;
    assert_eq!(b, ap.import_opening_bills(&s, company, &ap_input).await?);
    // The residuals are opening balances, not source-period transactions.
    assert!(ar
        .tax_totals(&s, company, &Default::default())
        .await?
        .is_empty());
    assert!(ap
        .tax_totals(&s, company, &Default::default())
        .await?
        .is_empty());
    assert!(ar
        .received_tax_totals(
            &s,
            company,
            &bilibi_finance_receivables::ReceivedTaxOptions::new(
                date(2026, 1, 1),
                date(2027, 12, 31)
            )
        )
        .await?
        .is_empty());
    assert!(ar
        .list_payments(&s, company, &Default::default())
        .await?
        .is_empty());
    assert!(ap
        .list_payments(&s, company, &Default::default())
        .await?
        .is_empty());
    let sst = services.get::<bilibi_finance_sst::Sst>(bilibi_finance_sst::SERVICE)?;
    assert!(sst
        .list_returns(&s, company, None, None, None)
        .await?
        .is_empty());
    // Retain source summaries through the shared reviewed operation. They are
    // evidence and cannot create a posting, return, payment or tax line.
    let registry = services.get::<Registry>("base/agent_api.registry")?;
    let key = "finance.reports.source_period.import";
    let Handler::SafeWrite(import) = registry.operations[key].definition.handler else {
        panic!("source schedule import must be a reviewed write");
    };
    for (kind, from, to, rows) in [
        (
            "comparative",
            "2025-01-01",
            "2025-12-31",
            json!([{"source_id":"cash","account_id":config["bank"],"side":"debit","amount":"100.00"}]),
        ),
        (
            "movement",
            "2026-06-01",
            "2026-06-30",
            json!([{"source_id":"sales","account_id":config["expense"],"side":"debit","amount":"20.00"}]),
        ),
        (
            "sst",
            "2026-03-01",
            "2026-04-30",
            json!([{"source_id":"return","authority":"Tax authority","reference":"FILED-1","event_date":"2026-05-15","amount":"10.00"}]),
        ),
        (
            "payroll",
            "2026-05-01",
            "2026-05-31",
            json!([{"source_id":"remittance","authority":"Payroll authority","reference":"PAID-1","event_date":"2026-06-15","amount":"8.00"}]),
        ),
    ] {
        let input = json!({"company_id":company,"kind":kind,"source_reference":format!("HISTORY-{kind}"),"source_system":"Source system","currency_code":"MYR","accounting_timezone":"Asia/Kuala_Lumpur","period_start":from,"period_end":to,"rows":rows});
        let mut raw = db.get().await?;
        let mut tx = l
            .write(
                &mut raw,
                &s,
                company,
                &[bilibi_finance_ledger::MANAGE_ACCOUNTS],
                "walkthrough.source-period",
            )
            .await?;
        let preview = import.preview(tx.sql(), &s, &input).await?.envelope(key);
        assert_eq!(
            preview["result"]["changes"]["rows"][0]["source_id"],
            rows[0]["source_id"]
        );
        assert_eq!(
            preview["result"]["changes"]["rows"][0]["amount"],
            rows[0]["amount"]
        );
        import.commit(&mut tx, &s, &input).await?;
        tx.commit().await?;
        let mut retry = db.get().await?;
        let tx = l
            .write(
                &mut retry,
                &s,
                company,
                &[bilibi_finance_ledger::MANAGE_ACCOUNTS],
                "walkthrough.source-period-refusal",
            )
            .await?;
        assert!(import.preview(tx.sql(), &s, &input).await.is_err());
        tx.rollback().await?;
    }
    assert_eq!(
        trial,
        l.trial_balance(&s, company, date(2026, 7, 1), None).await?
    );
    assert!(ar
        .tax_totals(&s, company, &Default::default())
        .await?
        .is_empty());
    assert!(ap
        .tax_totals(&s, company, &Default::default())
        .await?
        .is_empty());
    assert!(sst
        .list_returns(&s, company, None, None, None)
        .await?
        .is_empty());
    let asset_batch = assets
        .import_opening_register(&s, company, &asset_input)
        .await?;
    assert_eq!(
        asset_batch,
        assets
            .import_opening_register(&s, company, &asset_input)
            .await?
    );
    let reproduced_ar = ar
        .aged_receivables(
            &s,
            company,
            date(2026, 7, 1),
            &AgingOptions {
                through_entry_number: Some(before_ar.through_entry_number),
                through_opening_batch_id: Some(before_ar.through_opening_batch_id),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&before_ar)?,
        serde_json::to_value(&reproduced_ar)?
    );
    let reproduced_ap = ap
        .aged_payables(
            &s,
            company,
            date(2026, 7, 1),
            &bilibi_finance_payables::AgingQuery {
                through_entry_number: Some(before_ap.through_entry_number),
                through_opening_batch_id: Some(before_ap.through_opening_batch_id),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&before_ap)?,
        serde_json::to_value(&reproduced_ap)?
    );
    assert_eq!(
        before_ar_tie.total_difference,
        ar.tie_out_control_accounts(&s, company, date(2026, 7, 1), &before_ar_tie.pins())
            .await?
            .total_difference
    );
    assert_eq!(
        before_ap_tie.total_difference,
        ap.tie_out_control_accounts(&s, company, date(2026, 7, 1), &before_ap_tie.pins())
            .await?
            .total_difference
    );
    let reproduced_assets = assets
        .register_report(&s, company, date(2026, 7, 1), &before_assets.pins())
        .await?;
    assert!(reproduced_assets.rows.is_empty());
    assert_eq!(
        before_assets.reconciliation,
        reproduced_assets.reconciliation
    );
    assert_eq!(
        trial,
        l.trial_balance(&s, company, date(2026, 7, 1), None).await?
    );
    assert_eq!(
        ar.aged_receivables(&s, company, date(2026, 6, 30), &AgingOptions::default())
            .await?
            .total,
        rust_decimal::Decimal::ZERO
    );
    assert_eq!(
        ap.aged_payables(&s, company, date(2026, 6, 30), &Default::default())
            .await?
            .total,
        Money::ZERO
    );
    assert!(
        ar.tie_out_control_accounts(&s, company, date(2026, 7, 1), &Default::default())
            .await?
            .tied_out
    );
    assert!(
        ap.tie_out_control_accounts(&s, company, date(2026, 7, 1), &Default::default())
            .await?
            .tied_out
    );
    let aged = ar
        .aged_receivables(&s, company, date(2026, 7, 1), &Default::default())
        .await?;
    assert_eq!(aged.total.to_string(), "3000.0000");
    assert_eq!(aged.customers[0].documents[0].due_date, "2026-06-30");
    let register = assets
        .register_report(&s, company, date(2026, 7, 1), &ReportPins::default())
        .await?;
    assert!(register
        .reconciliation
        .iter()
        .all(|r| r.difference.is_zero()));
    assert_eq!(
        register.rows[0].accumulated.shown(trial.minor_units),
        "1800.00"
    );
    assert_eq!(
        register.rows[0].schedule[0].amount.shown(trial.minor_units),
        "106.25"
    );
    let run = assets.depreciate(&s, company, date(2026, 7, 1)).await?;
    assert_eq!(run.total.shown(trial.minor_units), "106.25");
    let register = assets
        .register_report(&s, company, date(2026, 7, 31), &ReportPins::default())
        .await?;
    assert!(register
        .reconciliation
        .iter()
        .all(|r| r.difference.is_zero()));
    assert_eq!(
        register.rows[0].accumulated.shown(trial.minor_units),
        "1906.25"
    );
    let docs = ar.list_documents(&s, company, &Default::default()).await?;
    let invoice = docs[0].id;
    let payment=ar.record_payment(&s,company,&json!({"series_id":config["payment_series"],"customer_id":config["customer_id"],"payment_date":"2026-07-15","amount":"1000","deposit_account_id":config["bank"],"allocations":[{"document_id":invoice,"amount":"1000"}]})).await?;
    // A later receipt of an opening residual also creates no source tax share.
    assert!(ar
        .received_tax_totals(
            &s,
            company,
            &bilibi_finance_receivables::ReceivedTaxOptions::new(
                date(2026, 1, 1),
                date(2027, 12, 31)
            )
        )
        .await?
        .is_empty());
    assert_eq!(
        ar.get_document(&s, company, invoice)
            .await?
            .outstanding
            .unwrap()
            .shown(trial.minor_units),
        "2000.00"
    );
    ar.reverse_payment(
        &s,
        company,
        payment.id,
        &json!({"reversal_date":"2026-07-16","reason":"Conversion settlement correction"}),
    )
    .await?;
    assert_eq!(
        ar.get_document(&s, company, invoice)
            .await?
            .outstanding
            .unwrap()
            .shown(trial.minor_units),
        "3000.00"
    );
    assert!(
        ar.tie_out_control_accounts(&s, company, date(2026, 7, 31), &Default::default())
            .await?
            .tied_out
    );
    let bill = ap.list_documents(&s, company, &Default::default()).await?[0].id;
    let imported_bill = ap.get_document(&s, company, bill).await?;
    assert_eq!(imported_bill.vendor_id, config["vendor_id"].as_i64());
    assert_eq!(
        imported_bill.party.key,
        config["vendor_key"].as_str().unwrap()
    );
    assert_eq!(
        imported_bill.opening.as_ref().unwrap()["vendor_name"],
        "Conversion vendor"
    );
    let note_on = |day| NewDocument {
        vendor_id: config["vendor_id"].as_i64(),
        party_document_number: Some("DEBIT-PROOF".into()),
        document_date: day,
        due_date: None,
        description: Some("Debit note conversion-date proof".into()),
        lines: vec![DocumentLineInput {
            description: "Supplier credit".into(),
            account_id: config["expense"].as_i64().expect("expense account"),
            amount: Money::parse("100", trial.minor_units).expect("fixture money"),
            tax_code_id: None,
            external_order_line_id: None,
            order_line_id: None,
            quantity: None,
        }],
        tax_lines: None,
    };
    match ap
        .create_debit_note(&manager, company, bill, &note_on(date(2026, 6, 15)))
        .await
    {
        Err(PayablesError::Invalid(fields)) => {
            let text = format!("{fields:?}");
            assert!(text.contains("document_date"), "{text}");
            assert!(
                text.contains("cannot be settled before the conversion date"),
                "{text}"
            );
        }
        other => panic!("debit note before conversion was not refused: {other:?}"),
    }
    assert!(matches!(
        ap.create_debit_note(&manager, company, 0, &note_on(date(2026, 7, 15)))
            .await,
        Err(PayablesError::DocumentNotFound)
    ));
    let note = ap
        .create_debit_note(&manager, company, bill, &note_on(date(2026, 7, 15)))
        .await?;
    ap.void_document(&manager, company, note.id, "Conversion-date proof only")
        .await?;
    let amount = Money::parse("500", trial.minor_units)?;
    let request = ap
        .submit_payment(
            &manager,
            company,
            &bilibi_finance_payables::NewPayment {
                payment_date: date(2026, 7, 15),
                vendor_id: config["vendor_id"].as_i64(),
                supplier_key: None,
                paid_from_account_id: config["bank"].as_i64().unwrap(),
                reference: Some("Opening settlement proof".into()),
                description: None,
                allocations: vec![bilibi_finance_payables::AllocationInput {
                    document_id: bill,
                    amount,
                }],
            },
        )
        .await?;
    let paid = ap.approve_payment(&s, company, request.id).await?;
    assert_eq!(
        ap.get_document(&s, company, bill)
            .await?
            .outstanding
            .unwrap()
            .shown(trial.minor_units),
        "1000.00"
    );
    assert!(
        ap.tie_out_control_accounts(&s, company, date(2026, 7, 31), &Default::default())
            .await?
            .tied_out
    );
    ap.reverse_payment(
        &s,
        company,
        paid.id,
        date(2026, 7, 16),
        Some("Conversion settlement correction".into()),
    )
    .await?;
    assert_eq!(
        ap.get_document(&s, company, bill)
            .await?
            .outstanding
            .unwrap()
            .shown(trial.minor_units),
        "1500.00"
    );
    assert!(
        ap.tie_out_control_accounts(&s, company, date(2026, 7, 31), &Default::default())
            .await?
            .tied_out
    );
    let context = l
        .opening_context_on(&**db.get().await?, &s, company)
        .await?;
    assert!(l
        .reverse_entry(
            &s,
            company,
            context.entry.id,
            &Reversal {
                entry_date: date(2026, 7, 31),
                description: "Refused while imports depend on opening".into(),
                reference: None
            }
        )
        .await
        .is_err());
    let c = db.get().await?;
    let entries: i64 = c
        .query_one(
            "SELECT count(*) FROM finance_ledger_entries WHERE tenant_id=1 AND company_id=$1",
            &[&company],
        )
        .await?
        .get(0);
    assert_eq!(entries, 6);
    for table in [
        "finance_receivables_opening_batches",
        "finance_payables_opening_batches",
        "finance_fixed_assets_opening_batches",
        "finance_ledger_opening_recognitions",
    ] {
        assert!(c
            .execute(
                &format!(
                    "UPDATE {table} SET company_id=company_id WHERE tenant_id=1 AND company_id=$1"
                ),
                &[&company]
            )
            .await
            .is_err());
    }
    println!(
        "Conversion facade proof: exact source balances, partial residuals, all three mismatch refusals, corrected preview, identical retries, independent approval, pre-cutoff exclusion, aged dates, control tie-outs, source accumulation differing from historical projection, prospective 106.25 depreciation, receipt, supplier payment and their reversals, guarded opening reversal, audit evidence and retained rows passed."
    );
    drop(c);
    conversion_lifecycle::prove(db, services, &config, [a, b, asset_batch], run.id).await?;
    Ok(())
}
#[tokio::main]
async fn main() -> Outcome<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    bilibi_database::migrate(&db, &bilibi_module_registry::discover()?).await?;
    let origin = std::env::var("APP_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:8147".into());
    let app = bilibi_web::App::new(db.clone(), &origin)?;
    bilibi_finance_walkthrough_fixture::without_step_up(&app.services)?;
    install(&db, &app.services).await?;
    // Identity-only fixture repair: the reviewer has a real Person affiliation,
    // so browser sessions can choose the current company. No business rows use SQL.
    db.get().await?.batch_execute("INSERT INTO persons(id,tenant_id,code,name,kind,email) VALUES(4,1,'person_4','Conversion reviewer','human','admin@example.test') ON CONFLICT DO NOTHING; INSERT INTO person_affiliations(id,tenant_id,person_id,organisation_id,title,work_email) VALUES(3,1,4,1,'Reviewer','admin@example.test') ON CONFLICT DO NOTHING; UPDATE users SET person_id=4,affiliation_id=3,organisation_id=1 WHERE tenant_id=1 AND id=4 AND person_id IS NULL;").await?;
    let admin = sign_in(&db, &app.services, 1, 4).await?;
    let grants = app
        .services
        .get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    grants
        .set_direct(&admin, 1, "admin.company.tenant-wide.manage", Some(true))
        .await?;
    if std::env::args().any(|s| s == "serve" || s == "prepare") {
        for company in [1, 2] {
            println!(
                "Disposable conversion source: {}",
                setup(&db, &app.services, company).await?
            );
        }
        if std::env::args().any(|s| s == "prepare") {
            return Ok(());
        }
        if std::env::args().any(|s| s == "assets-lifecycle") {
            bilibi_finance_walkthrough_fixture::assets_lifecycle::credentials(&db, &app.services)
                .await?;
        }
        println!(
            "Conversion browser fixture at {origin}; independent reviewer admin@example.test; walkthrough password. Company 001 UI first, Company 002 MCP second."
        );
        axum::serve(
            tokio::net::TcpListener::bind(std::env::var("BIND_ADDR")?).await?,
            bilibi_web::router(app),
        )
        .await?;
    } else {
        prove(&db, &app.services).await?;
    }
    Ok(())
}
