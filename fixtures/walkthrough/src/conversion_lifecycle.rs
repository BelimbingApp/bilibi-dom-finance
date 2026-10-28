//! Executable conversion lifecycle proof, composed with the actual owner services.
use bilibi_database::Db;
use bilibi_finance_fixed_assets::{DisposalInput, EventInput, FixedAssets};
use bilibi_finance_ledger::{Ledger, Money};
use bilibi_finance_payables::{AllocationInput, NewPayment, Payables};
use bilibi_finance_receivables::Receivables;
use bilibi_finance_walkthrough_fixture::sign_in;
use bilibi_module_registry::Services;
use jiff::civil::date;
use serde_json::{json, Value};
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
pub async fn prove(
    db: &Db,
    services: &Services,
    config: &Value,
    batches: [Value; 3],
    run_id: i64,
) -> Outcome<()> {
    let company = 3;
    let preparer = sign_in(db, services, 1, 1).await?;
    let approver = sign_in(db, services, 1, 4).await?;
    let ar = services.get::<Receivables>(bilibi_finance_receivables::SERVICE)?;
    let ap = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let assets = services.get::<FixedAssets>(bilibi_finance_fixed_assets::SERVICE)?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let day = date(2026, 8, 1);
    let units = ledger.get_book(&approver, company).await?.minor_units;
    let money = |text| Money::parse(text, units).expect("fixture money");
    let ar_before = ar
        .aged_receivables(&approver, company, day, &Default::default())
        .await?;
    let ap_before = ap
        .aged_payables(&approver, company, day, &Default::default())
        .await?;
    let invoice = ar_before.customers[0].documents[0].document_id;
    let bill = ap
        .list_documents(&approver, company, &Default::default())
        .await?
        .into_iter()
        .find(|d| d.document_type == "bill")
        .unwrap()
        .id;
    let asset = assets
        .register_report(&approver, company, day, &Default::default())
        .await?
        .rows[0]
        .asset
        .id;
    // 600 source rows exceed the former complete-register limit. Each chunk
    // is independently retained; none is settleable until final reconciliation.
    let header="source_id,customer_code,document_reference,document_date,due_date,original_total,outstanding,control_account_code\n";
    let chunk = |start, end| {
        let mut csv = header.to_owned();
        for n in start..end {
            csv.push_str(&format!(
                "corrected-{n},CONVERSION-CUSTOMER,CORRECTED-{n},2026-05-15,2026-06-30,5,5,1100\n"
            ));
        }
        csv
    };
    let common = json!({"source_reference":"AR-CORRECTED","currency_code":"MYR","series_id":config["invoice_series"],"replaces_batch_id":batches[0]["id"],"reason":"Replace incomplete source invoice detail","effective_date":"2026-08-01","part_count":2});
    let mut part1 = common.clone();
    part1["part"] = json!(1);
    part1["csv"] = json!(chunk(0, 300));
    let mut part2 = common.clone();
    part2["part"] = json!(2);
    part2["csv"] = json!(chunk(300, 600));
    assert_eq!(
        ar.stage_opening_import(&preparer, company, &part1).await?["state"],
        "committed"
    );
    assert_eq!(
        ar.stage_opening_import(&preparer, company, &part1).await?["already_staged"],
        true
    );
    let mut finalize_ar = common.clone();
    finalize_ar["staged"] = json!(true);
    finalize_ar["csv"] = json!("");
    assert!(ar
        .import_opening_invoices(&approver, company, &finalize_ar)
        .await
        .is_err());
    let mut changed = part1.clone();
    changed["csv"] = json!(chunk(0, 299));
    assert!(ar
        .stage_opening_import(&preparer, company, &changed)
        .await
        .is_err());
    changed = part1.clone();
    changed["part_count"] = json!(3);
    assert!(ar
        .stage_opening_import(&preparer, company, &changed)
        .await
        .is_err());
    ar.stage_opening_import(&preparer, company, &part2).await?;
    assert!(ar
        .import_opening_invoices(&preparer, company, &finalize_ar)
        .await
        .is_err());

    let mut part_ap = json!({"source_reference":"AP-CORRECTED","currency_code":"MYR","replaces_batch_id":batches[1]["id"],"reason":"Correct source bill original total","effective_date":"2026-08-01","part":1,"part_count":1,"csv":batches[1]["source_csv"].as_str().unwrap().replace(",2500,",",2600,")});
    ap.stage_opening_import(&preparer, company, &part_ap)
        .await?;
    part_ap["staged"] = json!(true);
    part_ap["csv"] = json!("");
    let mut part_assets = json!({"source_reference":"ASSETS-CORRECTED","currency_code":"MYR","replaces_batch_id":batches[2]["id"],"reason":"Correct remaining prospective life","effective_date":"2026-08-01","part":1,"part_count":1,"csv":batches[2]["source_csv"].as_str().unwrap().replace(",96,",",120,")});
    assets
        .stage_opening_import(&preparer, company, &part_assets)
        .await?;
    part_assets["staged"] = json!(true);
    part_assets["csv"] = json!("");

    // Corrections refuse standing dependent receipts and payments. Their own
    // audited reversal paths are the only way to release those dependencies.
    let receipt=ar.record_payment(&approver,company,&json!({"series_id":config["payment_series"],"customer_id":config["customer_id"],"payment_date":"2026-07-20","amount":"5","deposit_account_id":config["bank"],"allocations":[{"document_id":invoice,"amount":"5"}]})).await?;
    assert!(ar
        .import_opening_invoices(&approver, company, &finalize_ar)
        .await
        .is_err());
    ar.reverse_payment(
        &approver,
        company,
        receipt.id,
        &json!({"reversal_date":"2026-07-21","reason":"Release source correction"}),
    )
    .await?;
    let request = ap
        .submit_payment(
            &preparer,
            company,
            &NewPayment {
                payment_date: date(2026, 7, 20),
                vendor_id: config["vendor_id"].as_i64(),
                supplier_key: None,
                paid_from_account_id: config["bank"].as_i64().unwrap(),
                reference: None,
                description: None,
                allocations: vec![AllocationInput {
                    document_id: bill,
                    amount: money("5"),
                }],
            },
        )
        .await?;
    assert!(ap
        .import_opening_bills(&approver, company, &part_ap)
        .await
        .is_err());
    let paid = ap.approve_payment(&approver, company, request.id).await?;
    assert!(ap
        .import_opening_bills(&approver, company, &part_ap)
        .await
        .is_err());
    ap.reverse_payment(
        &approver,
        company,
        paid.id,
        date(2026, 7, 21),
        Some("Release source correction".into()),
    )
    .await?;
    assert!(assets
        .import_opening_register(&approver, company, &part_assets)
        .await
        .is_err());
    assets
        .dispose(
            &approver,
            company,
            asset,
            &DisposalInput {
                event: EventInput {
                    effective_date: date(2026, 7, 31),
                    reason: "Dependent disposal proof".into(),
                },
                proceeds: money("0"),
                settlement_account_id: None,
            },
        )
        .await?;
    assert!(assets
        .import_opening_register(&approver, company, &part_assets)
        .await
        .is_err());
    assets
        .reverse_disposal(
            &approver,
            company,
            asset,
            &EventInput {
                effective_date: day,
                reason: "Release source correction".into(),
            },
        )
        .await?;
    assets
        .reverse_run(
            &approver,
            company,
            run_id,
            &EventInput {
                effective_date: day,
                reason: "Release source correction".into(),
            },
        )
        .await?;
    let asset_before = assets
        .register_report(&approver, company, day, &Default::default())
        .await?;
    let before_entry = ledger
        .trial_balance(&approver, company, day, None)
        .await?
        .through_entry_number;

    // A late storage refusal after hundreds of inserted snapshots must roll
    // back the whole register, its recognition and all affected audit changes.
    db.get().await?.batch_execute("CREATE FUNCTION walkthrough_opening_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.description='Opening invoice CORRECTED-599' THEN RAISE EXCEPTION 'disposable late correction fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER walkthrough_opening_fault BEFORE INSERT ON finance_receivables_documents FOR EACH ROW EXECUTE FUNCTION walkthrough_opening_fault();").await?;
    assert!(ar
        .import_opening_invoices(&approver, company, &finalize_ar)
        .await
        .is_err());
    db.get().await?.batch_execute("DROP TRIGGER walkthrough_opening_fault ON finance_receivables_documents; DROP FUNCTION walkthrough_opening_fault();").await?;
    assert_eq!(
        ar.aged_receivables(&approver, company, day, &Default::default())
            .await?
            .customers[0]
            .documents[0]
            .document_id,
        invoice
    );
    let (left, right) = tokio::join!(
        ar.import_opening_invoices(&approver, company, &finalize_ar),
        ar.import_opening_invoices(&approver, company, &finalize_ar)
    );
    let replaced_ar = left?;
    assert_eq!(replaced_ar, right?);
    let replaced_ap = ap
        .import_opening_bills(&approver, company, &part_ap)
        .await?;
    let replaced_assets = assets
        .import_opening_register(&approver, company, &part_assets)
        .await?;
    assert_ne!(replaced_ar["id"], batches[0]["id"]);
    assert_ne!(replaced_ap["id"], batches[1]["id"]);
    assert_ne!(replaced_assets["id"], batches[2]["id"]);
    assert_eq!(
        ledger
            .trial_balance(&approver, company, day, None)
            .await?
            .through_entry_number,
        before_entry
    );
    let ar_after = ar
        .aged_receivables(&approver, company, day, &Default::default())
        .await?;
    assert_eq!(ar_after.customers[0].documents.len(), 600);
    assert_eq!(ar_after.total, ar_before.total);
    let old_ar_pins = ar_before.pins();
    // These pins preceded the extra payment/reversal proof too.
    let ar_old = ar
        .aged_receivables(
            &approver,
            company,
            day,
            &bilibi_finance_receivables::AgingOptions {
                through_entry_number: old_ar_pins.through_entry_number,
                through_opening_batch_id: old_ar_pins.through_opening_batch_id,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&ar_old)?,
        serde_json::to_value(&ar_before)?
    );
    let ap_old = ap
        .aged_payables(
            &approver,
            company,
            day,
            &bilibi_finance_payables::AgingQuery {
                through_entry_number: Some(ap_before.through_entry_number),
                through_opening_batch_id: Some(ap_before.through_opening_batch_id),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&ap_old)?,
        serde_json::to_value(&ap_before)?
    );
    let asset_old = assets
        .register_report(&approver, company, day, &asset_before.pins())
        .await?;
    assert_eq!(asset_old.rows[0].asset.id, asset);
    assert_eq!(asset_old.reconciliation, asset_before.reconciliation);
    assert_eq!(
        assets
            .register_report(&approver, company, date(2026, 7, 31), &Default::default())
            .await?
            .rows[0]
            .asset
            .id,
        asset
    );
    assert_ne!(
        assets
            .register_report(&approver, company, day, &Default::default())
            .await?
            .rows[0]
            .asset
            .id,
        asset
    );
    assert!(
        ar.tie_out_control_accounts(&approver, company, day, &Default::default())
            .await?
            .tied_out
    );
    assert!(
        ap.tie_out_control_accounts(&approver, company, day, &Default::default())
            .await?
            .tied_out
    );
    assert!(ar.record_payment(&approver,company,&json!({"series_id":config["payment_series"],"customer_id":config["customer_id"],"payment_date":"2026-08-01","amount":"5","deposit_account_id":config["bank"],"allocations":[{"document_id":invoice,"amount":"5"}]})).await.is_err());
    // Old source exports survive and identical historical retries remain safe.
    assert_eq!(ar.import_opening_invoices(&approver,company,&json!({"source_reference":batches[0]["source_reference"],"currency_code":"MYR","series_id":config["invoice_series"],"csv":batches[0]["source_csv"]})).await?,batches[0]);
    let run = assets
        .depreciate(&approver, company, date(2026, 8, 1))
        .await?;
    assert_eq!(run.total, money("170"));
    println!("Opening lifecycle: interrupted two-batch/600-invoice correction resumed; identical/concurrent retries, changed-source and self-approval refusals, late correction rollback, dependent receipts/payments/depreciation/disposal and owner reversals, accounting-date and original report boundaries, unchanged opening postings and prospective catch-up passed.");
    Ok(())
}
