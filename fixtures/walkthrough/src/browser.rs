//! NU1 browser records use the local authority without changing security grants.
use crate::clearance::{self, StandIn, CLIENT_ID, CLIENT_SECRET};
use bilibi_database::Db;
use bilibi_finance_einvoice::Einvoice;
use bilibi_module_registry::Services;
use bilibi_party_ids::OrganisationId;
use bilibi_queue::execution::{Config, Executor};
use bilibi_settings::Target;
use serde_json::json;
use std::sync::Arc;

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

pub async fn prepare_einvoice(
    db: &Db,
    services: &Services,
    manager: &bilibi_tenancy::Scope,
    admin: &bilibi_tenancy::Scope,
    company: i64,
) -> Outcome<tokio::sync::watch::Sender<bool>> {
    let target = Target::Company(OrganisationId::new(company)?);
    let url = clearance::serve(Arc::new(StandIn::default())).await?;
    clearance::save(
        db,
        services,
        admin,
        Target::Global,
        "finance.einvoice.sandbox.api_url",
        json!(url),
    )
    .await?;
    for (key, value) in [
        ("environment", "sandbox"),
        ("document_version", "1.0"),
        ("sandbox.client_id", CLIENT_ID),
        ("sandbox.client_secret", CLIENT_SECRET),
        ("supplier.tax_id", "C2584563222"),
        ("supplier.registration_number", "201901234567"),
        ("supplier.industry_code", "47111"),
        ("supplier.business_activity", "Retail"),
        ("supplier.phone", "+60123456789"),
        ("supplier.address_line1", "2 Jalan Contoh"),
        ("supplier.city", "Kuala Lumpur"),
        ("supplier.state_code", "14"),
        ("supplier.country_code", "MYS"),
    ] {
        clearance::save(
            db,
            services,
            manager,
            target,
            &format!("finance.einvoice.{key}"),
            json!(value),
        )
        .await?;
    }
    let facade = services.get::<Einvoice>(bilibi_finance_einvoice::SERVICE)?;
    let executor = Executor::from_services(services, Config::default())?;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    tokio::spawn(executor.run(shutdown));
    if facade
        .list_documents(
            manager,
            company,
            &bilibi_finance_einvoice::DocumentQuery::default(),
        )
        .await?
        .total_entries
        > 0
    {
        return Ok(stop);
    }
    for n in 1..=3 {
        let document = facade
            .issue_document(
                manager,
                company,
                &clearance::draft(&format!("BROWSER-E-{n}"), None),
            )
            .await?;
        if n == 1 {
            facade
                .submit_document(manager, company, document.document.id)
                .await?;
            let client = db.get().await?;
            clearance::jobs_settle(&client).await?;
        }
        println!(
            "E-invoice: /finance/einvoice/documents/{}",
            document.document.id
        );
    }
    Ok(stop)
}

/// Exercise the mounted pages against real facade records, without mocks.
pub async fn prove_messages(
    services: &Services,
    scope: &bilibi_tenancy::Scope,
    asset: i64,
) -> Outcome<()> {
    use bilibi_database::Error;
    use bilibi_tenancy::Scope;
    use bilibi_ui::page::{LiveInput, LiveRequest, PageRequest, PageService};
    use std::collections::HashMap;
    let pages = services.get::<PageService<Scope, Error>>("finance/fixed_assets.pages")?;
    for (operation, path, refusal) in [
        ("categories", HashMap::new(), json!({"type":"record_asset"})),
        (
            "asset",
            HashMap::from([("id".into(), asset.to_string())]),
            json!({"type":"dispose"}),
        ),
    ] {
        let mut state = pages
            .0
            .mount(PageRequest {
                operation,
                scope: Some(scope),
                path: &path,
                query: &HashMap::new(),
                body: &[],
                framed: false,
            })
            .await?
            .unwrap();
        let body = state
            .update(LiveRequest {
                scope,
                credential: "",
                input: LiveInput::Event(&refusal),
            })
            .await?;
        assert!(body.html.contains("id=\"flash-error\""));
        let refreshed = state
            .update(LiveRequest {
                scope,
                credential: "",
                input: LiveInput::Refresh,
            })
            .await?;
        assert_eq!(
            body.html, refreshed.html,
            "refresh retains refusal and draft"
        );
        let stale = json!({"type":"clear-flash","key":"success"});
        let unchanged = state
            .update(LiveRequest {
                scope,
                credential: "",
                input: LiveInput::Event(&stale),
            })
            .await?;
        assert_eq!(
            body.html, unchanged.html,
            "old success dismissal retains refusal"
        );
        let dismiss = json!({"type":"clear-flash","key":"error"});
        let dismissed = state
            .update(LiveRequest {
                scope,
                credential: "",
                input: LiveInput::Event(&dismiss),
            })
            .await?;
        assert!(!dismissed.html.contains("id=\"flash-error\""));
    }
    println!("NU1: Fixed Assets list/detail refusals survive refresh and stale success dismissal; matching dismissal clears without an error");
    Ok(())
}
