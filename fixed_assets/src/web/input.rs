//! Form decoding only; accounting validation remains in the facade.
use super::*;
use crate::{AcquisitionInput, Assumptions, DisposalInput, EventInput, Method, RevisionInput};
use bilibi_finance_ledger::{FieldError, MinorUnits, Money};
use bilibi_ui::{Choice, InputKind};
use jiff::civil::Date;
use rust_decimal::Decimal;

#[derive(Default, Clone)]
pub(super) struct Draft {
    pub values: HashMap<String, String>,
    pub errors: Vec<FieldError>,
}
impl Draft {
    pub fn posted(payload: &serde_json::Value) -> Self {
        Self {
            values: params::form_values(payload, "asset"),
            errors: vec![],
        }
    }
    pub fn value(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn field(
        &self,
        key: &str,
        label: &str,
        kind: InputKind,
        required: bool,
        prefix: &str,
    ) -> Field {
        let mut f = Field::in_form("asset", key)
            .id(format!("{prefix}-{}", key.replace('_', "-")))
            .label(label)
            .kind(kind)
            .value(self.value(key))
            .errors(
                self.errors
                    .iter()
                    .filter(|e| e.field == key)
                    .map(|e| e.message.clone())
                    .collect(),
            );
        if required {
            f = f.required();
        }
        f
    }
    pub fn method_field(&self) -> Field {
        let mut f = self
            .field(
                "method",
                "Default method",
                InputKind::Select,
                false,
                "asset",
            )
            .options(vec![
                Choice::new("Straight line", "straight_line"),
                Choice::new("Reducing balance", "reducing_balance"),
            ]);
        if self.value("method").is_empty() {
            f = f.value("straight_line");
        }
        f
    }
    pub fn id(&self, key: &'static str) -> crate::Result<i64> {
        params::positive_integer(Some(self.value(key)))
            .and_then(|v| i64::try_from(v).ok())
            .ok_or_else(|| {
                crate::invalid(
                    key,
                    if self.value(key).trim().is_empty() {
                        "can't be blank"
                    } else {
                        "is invalid"
                    },
                )
            })
    }
    pub fn optional_id(&self, key: &'static str) -> crate::Result<Option<i64>> {
        if self.value(key).trim().is_empty() {
            Ok(None)
        } else {
            self.id(key).map(Some)
        }
    }
    pub fn date(&self, key: &'static str) -> crate::Result<Date> {
        self.value(key).parse().map_err(|_| {
            crate::invalid(
                key,
                if self.value(key).is_empty() {
                    "can't be blank"
                } else {
                    "is invalid"
                },
            )
        })
    }
    fn decimal(&self, key: &'static str) -> crate::Result<Option<Decimal>> {
        if self.value(key).trim().is_empty() {
            return Ok(None);
        }
        self.value(key)
            .trim()
            .parse()
            .map(Some)
            .map_err(|_| crate::invalid(key, "is invalid"))
    }
    fn money(&self, key: &'static str, units: MinorUnits) -> crate::Result<Money> {
        Money::parse(self.value(key), units).map_err(|e| crate::invalid(key, e.to_string()))
    }
    pub fn revision(&self) -> crate::Result<RevisionInput> {
        let method = match self.value("method") {
            "" | "straight_line" => Method::StraightLine,
            "reducing_balance" => Method::ReducingBalance,
            _ => return Err(crate::invalid("method", "is invalid")),
        };
        Ok(RevisionInput {
            cost_account_id: self.id("cost_account_id")?,
            accumulated_account_id: self.id("accumulated_account_id")?,
            expense_account_id: self.id("expense_account_id")?,
            gain_account_id: self.id("gain_account_id")?,
            loss_account_id: self.id("loss_account_id")?,
            assumptions: Assumptions {
                method,
                life_months: i32::try_from(self.id("life_months")?)
                    .map_err(|_| crate::invalid("life_months", "is invalid"))?,
                annual_rate: self.decimal("annual_rate")?,
            },
            reason: self.value("reason").into(),
        })
    }
    pub fn acquisition(&self, units: MinorUnits) -> crate::Result<AcquisitionInput> {
        Ok(AcquisitionInput {
            category_revision_id: self.id("category_revision_id")?,
            description: self.value("description").into(),
            in_service_date: self.date("in_service_date")?,
            residual_value: if self.value("residual_value").trim().is_empty() {
                None
            } else {
                Some(self.money("residual_value", units)?)
            },
            method: None,
            life_months: None,
            annual_rate: None,
        })
    }
    pub fn event(&self) -> crate::Result<EventInput> {
        Ok(EventInput {
            effective_date: self.date("effective_date")?,
            reason: self.value("reason").into(),
        })
    }
    pub fn disposal(&self, units: MinorUnits) -> crate::Result<DisposalInput> {
        Ok(DisposalInput {
            event: self.event()?,
            proceeds: self.money("proceeds", units)?,
            settlement_account_id: self.optional_id("settlement_account_id")?,
        })
    }
    pub fn refused(&mut self, e: &FixedAssetsError) {
        self.errors = match e {
            FixedAssetsError::Invalid(f)
            | FixedAssetsError::Ledger(bilibi_finance_ledger::LedgerError::Invalid(f)) => f.clone(),
            _ => vec![],
        };
    }
}
pub(super) fn method(m: Method) -> &'static str {
    match m {
        Method::StraightLine => "straight_line",
        Method::ReducingBalance => "reducing_balance",
    }
}
