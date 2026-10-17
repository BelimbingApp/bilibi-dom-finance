//! Provision real OAuth credentials for the HTTP/MCP ledger walkthrough on
//! an explicitly disposable database. No book is opened here: the wire proof
//! starts from an empty ledger. Never print credentials or plan codes.
use bilibi_authz::grants::Grants;
use bilibi_core::agent_access::{
    code_challenge, AgentAccess, AuthorizeParams, CodeExchange, Decision, CLIENT_ID, OAUTH_CLIENTS,
};
use bilibi_database::{pool, Error, MutationBoundary};
use bilibi_finance_walkthrough_fixture::sign_in;
use bilibi_module_registry::Services;
use bilibi_settings::{Settings, Target};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::Arc,
    time::{Duration, SystemTime},
};

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_attachments_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_einvoice_roles!();

// H2 shares the credential bootstrap, never product seeds or production roles.
bilibi_finance_walkthrough_fixture::walkthrough_receivables_roles!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("BILIBI_AGENT_PROOF").as_deref() != Ok("disposable") {
        return Err(Error::Invalid.into());
    }
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_module_registry::verify_composition(
        &modules,
        bilibi_domains::MODULES,
        bilibi_extensions::MODULES,
    )?;
    bilibi_database::migrate(&db, &modules).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.install(&modules)?;
    if std::env::args().nth(1).as_deref() == Some("payables-control") {
        return payables_control(&db, &services).await;
    }
    let path = std::env::var("BILIBI_AGENT_CREDENTIALS")?;
    let origin = std::env::var("APP_ORIGIN")?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let admin = sign_in(&db, &services, 1, 4).await?;
    // A second real Person/Company affiliation, not an actor passed to a write.
    for key in bilibi_finance_ledger::CAPABILITIES
        .into_iter()
        .chain(["admin.company.tenant-wide.manage"])
    {
        grants.set_direct(&admin, 7, key, Some(true)).await?;
    }
    // H3: both real people can exercise Payables; this changes only the
    // explicitly disposable walkthrough database, never production seeds.
    for user in [1, 7] {
        for key in bilibi_finance_payables::CAPABILITIES {
            grants.set_direct(&admin, user, key, Some(true)).await?;
        }
    }
    // H2 grants are disposable fixture data. Include issuance and settlement,
    // which NR1's shared manager role deliberately does not carry.
    for user in [1, 7] {
        for key in bilibi_finance_receivables::CAPABILITIES {
            grants.set_direct(&admin, user, key, Some(true)).await?;
        }
    }
    let manager = sign_in(&db, &services, 1, 1).await?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let boundary = services.get::<MutationBoundary>("base/database.mutations")?;
    let mut client = db.get().await?;
    let mut tx = boundary
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.agent.clients"),
        )
        .await?;
    let callback = "https://example.test/ledger-proof/callback";
    settings
        .put(
            &mut tx,
            &admin,
            Target::Global,
            OAUTH_CLIENTS,
            json!({"ledger-proof":{"name":"Ledger walkthrough","redirect_uris":[callback]}}),
        )
        .await?;
    // Bound only this disposable proof, not production defaults.
    for key in [
        "agent_api.mcp_requests_per_minute",
        "agent_access.oauth_requests_per_minute",
    ] {
        settings
            .put(&mut tx, &admin, Target::Global, key, json!(10000))
            .await?;
    }
    tx.commit().await?;
    drop(client);
    let access = services.get::<AgentAccess>("core/agent_access.grants")?;
    let resource = format!("{}/mcp", origin.trim_end_matches('/'));
    let mut credentials = serde_json::Map::new();
    for (name, scope) in [
        ("manager", manager),
        ("approver", sign_in(&db, &services, 1, 7).await?),
        ("other", sign_in(&db, &services, 2, 5).await?),
    ] {
        let expiry = SystemTime::now() + Duration::from_secs(3600);
        let device = access.start_device(CLIENT_ID, "ledger-walkthrough").await?;
        access
            .decide(&scope, &device.user_code, Decision::Approve, expiry)
            .await?;
        let api = access.exchange(CLIENT_ID, &device.device_code).await?;
        let verifier = "ledger-walkthrough-verifier-with-at-least-forty-three-characters";
        let params = AuthorizeParams {
            client_id: "ledger-proof".into(),
            redirect_uri: callback.into(),
            response_type: "code".into(),
            resource: resource.clone(),
            scope: Some("bilibi".into()),
            code_challenge: code_challenge(verifier),
            code_challenge_method: "S256".into(),
            state: Some("ledger-walkthrough".into()),
        };
        let code = access.authorize(&scope, &params, &resource, expiry).await?;
        let mcp = access
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
        credentials.insert(
            name.into(),
            json!({"api":api.access_token,"mcp":mcp.access_token}),
        );
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(serde_json::to_string(&credentials)?.as_bytes())?;
    println!("Disposable fixture ready: empty ledgers; real manager, independent approver and other-tenant OAuth credentials saved privately.");
    Ok(())
}

// H3: real administrative facades for changing eligibility between wire
// preview and commit. Never writes business tables directly or reconciles
// grants, and refuses any database other than the installed disposable proof.
async fn payables_control(
    db: &bilibi_database::Db,
    services: &Services,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = db.get().await?;
    if c.query_opt(
        "SELECT 1 FROM users WHERE id=1 AND tenant_id=1 AND email='manager@example.test'",
        &[],
    )
    .await?
    .is_none()
    {
        return Err(Error::Invalid.into());
    }
    drop(c);
    let args: Vec<_> = std::env::args().skip(2).collect();
    let admin = sign_in(db, services, 1, 4).await?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["grant", key, enabled]
            if bilibi_finance_payables::CAPABILITIES.contains(key)
                || [
                    bilibi_finance_ledger::VIEW,
                    bilibi_finance_ledger::POST_ENTRIES,
                ]
                .contains(key) =>
        {
            let enabled: bool = enabled.parse()?;
            services
                .get::<Grants>("base/authz.grants")?
                .set_direct(&admin, 1, key, Some(enabled))
                .await?;
        }
        ["duties", company, required] => {
            let company: i64 = company.parse()?;
            if ![1, 2].contains(&company) {
                return Err(Error::Invalid.into());
            }
            services
                .get::<bilibi_finance_ledger::Ledger>(bilibi_finance_ledger::SERVICE)?
                .set_duty_separation(
                    &admin,
                    company,
                    required.parse()?,
                    Some("Disposable H3 small-team control"),
                )
                .await?;
        }
        ["archive"] => {
            services
                .get::<bilibi_company::CompanyService>("core/company.companies")?
                .archive_company(
                    &admin,
                    2,
                    Some("Disposable H3 archive between review and commit"),
                )
                .await?;
        }
        _ => return Err(Error::Invalid.into()),
    }
    println!("Disposable H3 administrative control recorded through its facade.");
    Ok(())
}
