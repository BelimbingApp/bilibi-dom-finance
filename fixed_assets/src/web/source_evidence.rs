//! One rendering for reviewed imports and the asset's retained source evidence.
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::{MinorUnits, Money};
use bilibi_ui::{Alert, BuiltinColumn, Card, Cell, FlexRow, FlexTable, Html, ListColumns};
use serde_json::Value;

#[derive(Default)]
pub(super) struct EvidenceView {
    tables: std::collections::BTreeMap<String, ListColumns>,
}
impl EvidenceView {
    pub(super) fn handle(
        &mut self,
        event: &str,
        values: &std::collections::HashMap<String, String>,
    ) {
        if let Some(columns) = event
            .strip_prefix("source_evidence_grid:")
            .and_then(|id| self.tables.get_mut(id))
        {
            columns.handle(values);
        }
    }
    pub(super) fn render(
        &mut self,
        source: &str,
        evidence: &Value,
        units: MinorUnits,
    ) -> Result<Html> {
        if evidence.is_null() {
            return Ok(Html::EMPTY);
        }
        let dom = format!(
            "source-evidence-{}",
            source
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let policy_dom = format!("{dom}-policy");
        let mut body = Html::render(&Alert {
        id: &policy_dom,
        body: Html::text("Source history and tax schedules are retained evidence. They create no posting, change no prospective estimate and are separate from subsequent depreciation activity."),
        ..Default::default()
    }).map_err(|_| Error::Internal)?;
        for (kind, label, fields) in [
            (
                "history",
                "Source history",
                vec![
                    ("kind", "Event"),
                    ("effective_date", "Source date"),
                    ("amount", "Amount"),
                    ("residual_value", "Residual value"),
                    ("remaining_life_months", "Remaining months"),
                    ("method", "Source method"),
                    ("annual_rate", "Source annual rate (%)"),
                    ("reference", "Source reference"),
                    ("note", "Note"),
                ],
            ),
            (
                "source_period_expense",
                "Source-period expense (not subsequent activity)",
                vec![
                    ("from", "From"),
                    ("through", "Through"),
                    ("amount", "Source expense"),
                    ("reference", "Source reference"),
                ],
            ),
            (
                "tax_capital_allowances",
                "Tax capital-allowance evidence (separate from book depreciation)",
                vec![
                    ("from", "From"),
                    ("through", "Through"),
                    ("qualifying_expenditure", "Qualifying expenditure"),
                    ("allowances", "Tax allowances"),
                    ("reference", "Source reference"),
                ],
            ),
        ] {
            let rows: Vec<_> = evidence[kind]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(index, row)| {
                    let mut result = FlexRow::new(format!("{source}-{kind}-{index}"));
                    for (key, _) in &fields {
                        let value = if [
                            "amount",
                            "residual_value",
                            "qualifying_expenditure",
                            "allowances",
                        ]
                        .contains(key)
                        {
                            row[key]
                                .as_str()
                                .and_then(|s| Money::parse(s, units).ok())
                                .map(|m| m.display(units))
                                .unwrap_or_default()
                        } else {
                            row[key].as_str().map(str::to_owned).unwrap_or_else(|| {
                                if row[key].is_null() {
                                    String::new()
                                } else {
                                    row[key].to_string()
                                }
                            })
                        };
                        result = result.cell(*key, Cell::text(value));
                    }
                    result
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            let table_dom = format!("{dom}-{kind}");
            if !self.tables.contains_key(&table_dom) {
                self.tables.insert(
                    table_dom.clone(),
                    ListColumns::mount(
                        &table_dom,
                        fields
                            .iter()
                            .map(|(key, label)| BuiltinColumn::new(key, label))
                            .collect(),
                    )
                    .map_err(|_| Error::Internal)?,
                );
            }
            let columns = &self.tables[&table_dom];
            let event = format!("source_evidence_grid:{table_dom}");
            let mut table = FlexTable::new(&table_dom, columns.flex_columns(), rows);
            table.event = &event;
            table.zoom = columns.zoom;
            table.suggestions = columns.flex_suggestions();
            table.add_query = &columns.add_query;
            table.caption = label;
            body.push(&Html::render(&table).map_err(|_| Error::Internal)?);
        }
        Html::render(&Card {
            id: &dom,
            title: &format!("Source evidence: {source}"),
            body,
            ..Default::default()
        })
        .map_err(|_| Error::Internal)
    }
}
