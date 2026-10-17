//! Finance's private normalized accounting inputs, mirroring Bilimbi
//! Payroll.PeopleSource. These are NOT a People event wire contract.
use crate::{PayrollError, Result};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use rust_decimal::Decimal;

pub(crate) struct Component {
    pub code: String,
    pub direction: String,
    pub amount: Decimal,
}
pub(crate) struct Run {
    pub id: i64,
    pub reference: String,
    pub currency_code: String,
    pub pay_date: Option<Date>,
    pub finalised: bool,
    pub components: Vec<Component>,
}
pub(crate) struct Claim {
    pub id: i64,
    pub reference: String,
    pub currency_code: String,
    pub approved_on: Option<Date>,
    pub amount: Decimal,
    pub category_code: Option<String>,
}

// TODO(domain events): consume People publisher-documented run/claim facts
// into an owned projection once that contract exists. Never call People
// through ports, read its tables, or accept caller-supplied accounting facts.
pub(crate) fn available() -> bool {
    false
}
pub(crate) async fn runs(_: &Scope, _: i64) -> Result<Vec<serde_json::Value>> {
    Err(PayrollError::PeopleUnavailable)
}
pub(crate) async fn run(_: &Scope, _: i64, _: i64) -> Result<Run> {
    Err(PayrollError::PeopleUnavailable)
}
pub(crate) async fn claims(_: &Scope, _: i64) -> Result<Vec<Claim>> {
    Err(PayrollError::PeopleUnavailable)
}
