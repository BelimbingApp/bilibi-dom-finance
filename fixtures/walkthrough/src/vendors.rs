//! Reusable linked vendor setup for disposable Finance walkthroughs. Product
//! code never creates a company from historical supplier evidence.
use bilibi_database::Db;
use bilibi_module_registry::Services;
use bilibi_tenancy::Scope;

pub async fn vendor(
    _db: &Db,
    services: &Services,
    scope: &Scope,
    company: i64,
    code: &str,
    name: &str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let p = services.get::<bilibi_finance_payables::Payables>(bilibi_finance_payables::SERVICE)?;
    if let Some(v) = p
        .list_vendors(scope, company, 0, 500)
        .await?
        .into_iter()
        .find(|v| v.code == code)
    {
        return Ok(v.id);
    }
    let core = services.get::<bilibi_company::CompanyService>(bilibi_company::SERVICE)?;
    let created = match core
        .create_company(
            scope,
            &bilibi_company::NewCompany {
                name: format!("{name} ({code})"),
                status: Some("active".into()),
                ..bilibi_company::NewCompany::default()
            },
        )
        .await?
    {
        bilibi_company::CreateOutcome::Created(c) => c,
        bilibi_company::CreateOutcome::Refused(e) => {
            return Err(format!("fixture company refused: {e:?}").into())
        }
    };
    Ok(p.create_vendor(
        scope,
        company,
        &bilibi_finance_payables::VendorInput {
            code: code.into(),
            core_company_id: created.id,
            payment_terms_days: 0,
            payable_account_id: None,
            expense_account_id: None,
            tax_code_id: None,
            active: true,
        },
    )
    .await?
    .id)
}
