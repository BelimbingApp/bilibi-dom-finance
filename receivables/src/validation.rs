//! Bilimbi Receivables changesets: cast only editable fields, then validate
//! effective values. Calculated and immutable attributes never enter SQL.
use crate::{invalid, Result};
use bilibi_finance_ledger::{Account, AccountType};
use rust_decimal::Decimal;
use serde_json::{json, Value};

pub(crate) fn merge(old: Option<&Value>, attrs: &Value, fields: &[&str]) -> Result<Value> {
    let attrs = attrs
        .as_object()
        .ok_or_else(|| invalid("attributes", "is invalid"))?;
    let mut value = old.cloned().unwrap_or_else(|| json!({}));
    for field in fields {
        if let Some(v) = attrs.get(*field) {
            value[*field] = v.clone();
        }
    }
    Ok(value)
}
pub(crate) fn text(data: &mut Value, field: &str, required: bool, max: usize) -> Result<()> {
    let value = match &data[field] {
        Value::Null => None,
        Value::String(s) => Some(s.trim()).filter(|s| !s.is_empty()),
        _ => return Err(invalid(field, "is invalid")),
    };
    if required && value.is_none() {
        return Err(invalid(field, "can't be blank"));
    }
    if value.is_some_and(|s| s.chars().count() > max) {
        return Err(invalid(
            field,
            format!("should be at most {max} character(s)"),
        ));
    }
    data[field] = value.map_or(Value::Null, |s| json!(s));
    Ok(())
}
pub(crate) fn integer(data: &mut Value, field: &str, required: bool) -> Result<Option<i64>> {
    let v = match &data[field] {
        Value::Null => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(s.parse().map_err(|_| invalid(field, "is invalid"))?),
        Value::Number(n) => Some(n.as_i64().ok_or_else(|| invalid(field, "is invalid"))?),
        _ => return Err(invalid(field, "is invalid")),
    };
    if required && v.is_none() {
        return Err(invalid(field, "can't be blank"));
    }
    data[field] = json!(v);
    Ok(v)
}
pub(crate) fn range(data: &mut Value, field: &str, min: i64, max: i64) -> Result<()> {
    let n = integer(data, field, true)?.expect("required integer");
    if n < min {
        return Err(invalid(
            field,
            format!("must be greater than or equal to {min}"),
        ));
    }
    if n > max {
        return Err(invalid(
            field,
            format!("must be less than or equal to {max}"),
        ));
    }
    Ok(())
}
pub(crate) fn flag(data: &mut Value, field: &str) -> Result<()> {
    let value = match &data[field] {
        Value::Bool(v) => *v,
        Value::String(s) if s == "true" || s == "1" => true,
        Value::String(s) if s == "false" || s == "0" => false,
        Value::Null => return Err(invalid(field, "can't be blank")),
        _ => return Err(invalid(field, "is invalid")),
    };
    data[field] = json!(value);
    Ok(())
}
pub(crate) fn choice(data: &mut Value, field: &str, choices: &[&str]) -> Result<()> {
    text(data, field, true, usize::MAX)?;
    if !choices.contains(&data[field].as_str().expect("validated text")) {
        return Err(invalid(field, "is invalid"));
    }
    Ok(())
}
pub(crate) fn decimal(
    data: &mut Value,
    field: &str,
    positive: bool,
    limit: Decimal,
) -> Result<Decimal> {
    let text = match &data[field] {
        Value::String(s) if !s.trim().is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null | Value::String(_) => return Err(invalid(field, "can't be blank")),
        _ => return Err(invalid(field, "is invalid")),
    };
    let n = Decimal::from_str_exact(&text)
        .map_err(|_| invalid(field, "is invalid"))?
        .normalize();
    if n < Decimal::ZERO || (positive && n.is_zero()) {
        return Err(invalid(
            field,
            if positive {
                "must be greater than 0"
            } else {
                "must be greater than or equal to 0"
            },
        ));
    }
    if n >= limit {
        return Err(invalid(
            field,
            format!(
                "must be less than {}",
                if limit == Decimal::from(10_000_000_000_000_000_i64) {
                    "1E+16".into()
                } else {
                    limit.to_string()
                }
            ),
        ));
    }
    if n.scale() > 4 {
        return Err(invalid(field, "has more than 4 decimal places"));
    }
    data[field] = json!(n.to_string());
    Ok(n)
}
pub(crate) fn date(data: &mut Value, field: &str) -> Result<()> {
    if data[field].is_null() || data[field] == "" {
        data[field] = Value::Null;
        return Ok(());
    }
    let date: jiff::civil::Date = data[field]
        .as_str()
        .ok_or_else(|| invalid(field, "is invalid"))?
        .parse()
        .map_err(|_| invalid(field, "is invalid"))?;
    data[field] = json!(date.to_string());
    Ok(())
}
pub(crate) fn account(
    data: &mut Value,
    field: &str,
    chart: &[Account],
    kind: AccountType,
) -> Result<()> {
    let id = integer(data, field, true)?.expect("required id");
    let Some(account) = chart.iter().find(|a| a.id == id) else {
        return Err(invalid(field, "is not a ledger account of this company"));
    };
    if !account.active {
        return Err(invalid(field, "is an inactive ledger account"));
    }
    if account.account_type != kind {
        return Err(invalid(
            field,
            format!("must be a ledger {} account", kind.as_str()),
        ));
    }
    Ok(())
}
