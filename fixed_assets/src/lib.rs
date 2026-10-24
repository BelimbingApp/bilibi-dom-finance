//! Bilimbi.Finance.FixedAssets: immutable category defaults and acquisition
//! assumptions, monthly runs and immutable corrections. Registration recognises
//! an existing debit; it never posts or takes ownership of it.
mod acquisitions;
mod activity;
mod categories;
mod contributions;
mod reports;
mod schedule;
mod store;
mod web;

pub use activity::{DisposalInput, Event, EventAction, EventInput, Run, RunException, RunItem};
use bilibi_database::{Db, Error, MutationTransaction};
use bilibi_finance_ledger::{FieldError, Ledger, LedgerError, Money};
use bilibi_finance_payables::{Payables, PayablesError};
use bilibi_module_registry::{Layer, Module, Service};
use bilibi_tenancy::Scope;
pub use contributions::{
    ACQUIRE, CAPABILITIES, CONFIGURE, DEPRECIATE, DISPOSE, REVERSE, VIEW, VOID,
};
use jiff::civil::Date;
pub use reports::{Reconciliation, ReconciliationKind, RegisterReport, RegisterRow, ReportPins};
use rust_decimal::Decimal;
use serde::Serialize;
use std::sync::Arc;
pub use store::EvidencePage;
use tokio_postgres::GenericClient;

pub const MODULE: &str = "finance/fixed_assets";
pub const SERVICE: &str = "finance/fixed_assets.fixed_assets";
inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/agent_api", "base/authz", "base/database", "base/tenancy", "base/datetime", "base/locale", "base/menu", "base/ui", "core/company", "finance/ledger", "finance/payables"],
    migrations: &[
        ("finance-fixed-assets-001-acquisitions", include_str!("../migrations/001-acquisitions.sql")),
        ("finance-fixed-assets-002-runs-corrections", include_str!("../migrations/002-runs-corrections.sql")),
    ],
    services: &[Service { id: SERVICE, factory: |s| Ok(Arc::new(FixedAssets {
        db: s.get::<Db>("base/database.pool")?.as_ref().clone(),
        ledger: s.get(bilibi_finance_ledger::SERVICE)?,
        payables: s.get(bilibi_finance_payables::SERVICE)?,
    })) }, Service { id: web::PAGES, factory: |s| Ok(Arc::new(bilibi_ui::page::PageService::<Scope, Error>(Arc::new(web::Pages {
        contracts: s.get(bilibi_agent_api::page::SERVICE)?,
        db: s.get::<Db>("base/database.pool")?.as_ref().clone(),
        assets: s.get(SERVICE)?,
        ledger: s.get(bilibi_finance_ledger::SERVICE)?,
        display: s.get("base/datetime.display")?,
        locales: s.get("base/locale.locales")?,
    })))) }],
} }

pub type Result<T> = std::result::Result<T, FixedAssetsError>;
#[derive(Debug, thiserror::Error)]
pub enum FixedAssetsError {
    #[error("You do not have permission for this action.")]
    Unauthorized,
    #[error("Company not found")]
    CompanyNotFound,
    #[error("This company is archived and cannot be changed.")]
    CompanyArchived,
    #[error("Not found")]
    NotFound,
    #[error("Bill not posted")]
    BillNotPosted,
    #[error("Bill line not found")]
    BillLineNotFound,
    #[error("Entry line not found")]
    EntryLineNotFound,
    #[error("Not acquisition debit")]
    NotAcquisitionDebit,
    #[error("The acquisition entry was reversed. Void the asset's acquisition from its page.")]
    AcquisitionReversed,
    #[error("Invalid period")]
    InvalidPeriod,
    #[error("Open a fiscal year covering this month before running depreciation.")]
    PeriodNotFound,
    #[error("The period is closed. Choose an open period.")]
    PeriodClosed,
    #[error("This date precedes recorded asset activity. Choose a later date.")]
    LaterActivity,
    #[error("Reverse later depreciation runs and standing disposals first.")]
    DependentPostings,
    #[error("Empty run")]
    EmptyRun,
    #[error("Already reversed")]
    AlreadyReversed,
    #[error("Before original")]
    BeforeOriginal,
    #[error("Before acquisition")]
    BeforeAcquisition,
    #[error("Already disposed")]
    AlreadyDisposed,
    #[error("Not disposed")]
    NotDisposed,
    #[error("Post depreciation through the last completed month before disposing of this asset.")]
    DepreciationRequired,
    #[error("Invalid settlement account")]
    InvalidSettlementAccount,
    #[error("This asset's acquisition was voided, so it cannot be disposed of.")]
    AssetVoided,
    #[error("This asset's acquisition is already voided.")]
    AlreadyVoided,
    #[error("Only an asset whose acquisition entry was reversed in the ledger can be voided.")]
    AcquisitionNotReversed,
    #[error("Reverse the standing disposal before voiding this asset's acquisition.")]
    DisposalStanding,
    #[error("Invalid option")]
    InvalidOption,
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Payables(#[from] PayablesError),
    #[error(transparent)]
    Failed(#[from] Error),
}
impl From<tokio_postgres::Error> for FixedAssetsError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Failed(e.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    StraightLine,
    ReducingBalance,
}

/// Full assumptions, required on each category revision. The annual percentage
/// is required only for reducing balance; straight line always clears it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Assumptions {
    pub method: Method,
    pub life_months: i32,
    pub annual_rate: Option<Decimal>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RevisionInput {
    pub cost_account_id: i64,
    pub accumulated_account_id: i64,
    pub expense_account_id: i64,
    pub gain_account_id: i64,
    pub loss_account_id: i64,
    #[serde(flatten)]
    pub assumptions: Assumptions,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub struct CategoryRevision {
    pub id: i64,
    pub category_id: i64,
    pub name: String,
    pub defaults: RevisionInput,
    pub recorded_by_user_id: i64,
    pub recorded_by_impersonator_id: Option<i64>,
    pub recorded_at: String,
}
#[derive(Clone, Debug)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub revision: CategoryRevision,
    pub revisions: Vec<CategoryRevision>,
}
/// Only registration-time overrides. Cost, acquisition date and source numbers
/// are never caller input. None inherits a revision's assumption (or zero residual).
#[derive(Clone, Debug)]
pub struct AcquisitionInput {
    pub category_revision_id: i64,
    pub description: String,
    pub in_service_date: Date,
    pub residual_value: Option<Money>,
    pub method: Option<Method>,
    pub life_months: Option<i32>,
    pub annual_rate: Option<Decimal>,
}
#[derive(Clone, Debug)]
pub struct Asset {
    pub id: i64,
    pub asset_number: i64,
    pub category_revision_id: i64,
    pub description: String,
    pub acquisition_date: Date,
    pub in_service_date: Date,
    pub cost: Money,
    pub residual_value: Money,
    pub assumptions: Assumptions,
    pub entry_id: i64,
    pub entry_number: i64,
    pub entry_line_number: i32,
    pub bill_id: Option<i64>,
    pub bill_line_number: Option<i32>,
    pub recorded_by_user_id: i64,
    pub recorded_by_impersonator_id: Option<i64>,
    pub recorded_at: String,
}
#[derive(Clone, Debug)]
pub struct ScheduleRow {
    pub period: Date,
    pub amount: Money,
    pub accumulated: Money,
    pub carrying_value: Money,
}
/// Posted charge with the earliest correction that reversed it.
#[derive(Clone, Debug)]
pub struct AssetCharge {
    pub run_id: i64,
    pub period: Date,
    pub amount: Money,
    pub entry_number: Option<i64>,
    pub recorded_by_user_id: i64,
    pub recorded_by_impersonator_id: Option<i64>,
    pub recorded_at: String,
    pub reversal: Option<Event>,
}
#[derive(Clone, Debug)]
pub struct AssetDetail {
    pub asset: Asset,
    pub revision: CategoryRevision,
    pub schedule: Vec<ScheduleRow>,
    pub charges: Vec<AssetCharge>,
    pub events: Vec<Event>,
    pub voided: bool,
}
impl AssetDetail {
    /// Scheduled (not posted) depreciation effective at month-end. A mid-month
    /// date excludes that month's full charge.
    pub fn scheduled_depreciation(&self, as_of: Date) -> Money {
        self.schedule
            .iter()
            .filter(|r| r.period.last_of_month() <= as_of)
            .map(|r| r.amount)
            .sum()
    }
}

pub struct FixedAssets {
    db: Db,
    ledger: Arc<Ledger>,
    payables: Arc<Payables>,
}
impl FixedAssets {
    async fn write<'c>(
        &self,
        c: &'c mut tokio_postgres::Client,
        s: &Scope,
        company: i64,
        cap: &str,
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        self.ledger
            .write(c, s, company, &[cap], action)
            .await
            .map_err(|e| match e {
                LedgerError::Unauthorized => FixedAssetsError::Unauthorized,
                LedgerError::CompanyNotFound => FixedAssetsError::CompanyNotFound,
                LedgerError::CompanyArchived => FixedAssetsError::CompanyArchived,
                e => FixedAssetsError::Ledger(e),
            })
    }
    pub fn evidence_datasets(&self) -> Vec<&'static str> {
        store::datasets()
    }
    pub async fn evidence_page(
        &self,
        s: &Scope,
        company: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.evidence_page_on(&**c, s, company, dataset, limit, after)
            .await
    }

    /// Authorized evidence on the caller's snapshot; never opens another connection.
    pub async fn evidence_page_on(
        &self,
        c: &(impl tokio_postgres::GenericClient + Sync),
        s: &Scope,
        company: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        authorize(c, s, company, VIEW).await?;
        store::page(c, s.tenant(), company, dataset, limit, after).await
    }
}
async fn authorize(c: &impl GenericClient, s: &Scope, company: i64, cap: &str) -> Result<()> {
    bilibi_company::authorize_target(c, s, company, cap)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            Error::Forbidden | Error::Unauthorized => FixedAssetsError::Unauthorized,
            Error::NotFound => FixedAssetsError::CompanyNotFound,
            e => FixedAssetsError::Failed(e),
        })
}
fn invalid(field: &'static str, message: impl Into<String>) -> FixedAssetsError {
    FixedAssetsError::Invalid(vec![FieldError::new(field, message)])
}
fn text(input: &str, field: &'static str, max: usize) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(invalid(field, "can't be blank"));
    }
    if input.chars().count() > max {
        return Err(invalid(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    Ok(input.into())
}
