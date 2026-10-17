//! Mirrors Bilimbi `Finance.Reconciliation.Web.BankAccountsLive`.
use super::{company, event_type, flash_map, format::told, record_id, render, Pages, DENIED};
use crate::{
    BankAccount, BankAccountEdit, FieldError, NewBankAccount, ReconciliationError,
    MANAGE_BANK_ACCOUNTS,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::{Account, AccountType, LedgerError};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::*, params, Badge, BadgeKind, BuiltinColumn, Cell, Choice, EmptyState, Field, FlashMap,
    FlexRow, FlexTable, Html, InputKind, ListColumns, ListSpec, ListState, StatusKind, TableEmpty,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("bank accounts list")
});
const MAPPING: [&str; 12] = [
    "delimiter",
    "header_rows",
    "date",
    "date_format",
    "description",
    "reference",
    "amount",
    "amount_sign",
    "debit",
    "credit",
    "balance",
    "decimal_separator",
];
fn blank() -> HashMap<String, String> {
    [
        ("name", ""),
        ("ledger_account_id", ""),
        ("charges_account_id", ""),
        ("interest_account_id", ""),
        ("match_window_days", "5"),
        ("delimiter", ","),
        ("header_rows", "1"),
        ("date", ""),
        ("date_format", "YYYY-MM-DD"),
        ("description", ""),
        ("reference", ""),
        ("amount", ""),
        ("amount_sign", "deposits_positive"),
        ("debit", ""),
        ("credit", ""),
        ("balance", ""),
        ("decimal_separator", "."),
        ("reason", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}
fn columns() -> Vec<BuiltinColumn> {
    [
        ("name", "Name"),
        ("ledger", "Ledger account"),
        ("charges", "Charges to"),
        ("interest", "Interest to"),
        ("window", "Window"),
        ("status", "Status"),
    ]
    .into_iter()
    .map(|(k, v)| BuiltinColumn::new(k, v))
    .collect()
}
pub(super) struct Index {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    editing: Option<BankAccount>,
    draft: HashMap<String, String>,
    errors: Vec<FieldError>,
    flash: Option<(StatusKind, String)>,
}
#[derive(Template)]
#[template(path = "bank_accounts.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    table: Option<Html>,
    forbidden: Html,
    chart_forbidden: Html,
    manage_forbidden: Html,
    fields: Option<Vec<Field>>,
    editing: bool,
    heading: String,
    ledger_fixed: String,
}
#[derive(Template)]
#[template(
    source = r#"{% import "components.html" as ui %}<a id="bank-account-{{ account.id }}-matching" href="/finance/reconciliation/accounts/{{ account.id }}/matching">Matching</a>{% if manage %}{% call ui::button(id=edit_id, event="edit", name="id", value=account.id.to_string()) %}Edit{% endcall %}{% call ui::button(id=toggle_id, event="toggle_active", name="id", value=account.id.to_string()) %}{% if account.active %}Deactivate{% else %}Activate{% endif %}{% endcall %}{% endif %}"#,
    ext = "html"
)]
struct Actions<'a> {
    account: &'a BankAccount,
    manage: bool,
    edit_id: String,
    toggle_id: String,
}

impl Index {
    pub(super) fn new(pages: &Pages, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            pages: pages.clone(),
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Invalid)?,
            columns: ListColumns::mount("bank-accounts", columns()).map_err(|_| Error::Internal)?,
            editing: None,
            draft: blank(),
            errors: vec![],
            flash: None,
        })
    }
    fn get(&self, key: &str) -> String {
        self.draft.get(key).cloned().unwrap_or_default()
    }
    fn field(&self, key: &str, dom: &str, label: &str) -> Field {
        Field::in_form("bank_account", key)
            .id(dom)
            .label(label)
            .value(self.get(key))
            .errors(
                self.errors
                    .iter()
                    .filter(|e| e.field == key || (key == "date" && e.field == "import_mapping"))
                    .map(|e| e.message.clone())
                    .collect(),
            )
    }
    fn fields(&self, chart: &[Account]) -> Vec<Field> {
        let choice = |kind| {
            chart
                .iter()
                .filter(|a| a.active && a.account_type == kind)
                .map(|a| Choice::new(format!("{} {}", a.code, a.name), a.id.to_string()))
                .collect()
        };
        let mut fields = vec![self
            .field("name", "bank-account-name", "Name")
            .attr("maxlength", "200")
            .required()];
        if self.editing.is_none() {
            fields.push(
                self.field(
                    "ledger_account_id",
                    "bank-account-ledger-account",
                    "Ledger account (asset)",
                )
                .kind(InputKind::Select)
                .prompt("Choose an account")
                .options(choice(AccountType::Asset))
                .required(),
            );
        }
        fields.extend([
            self.field("charges_account_id","bank-account-charges-account","Bank charges post to (expense)").kind(InputKind::Select).prompt("Not set").options(choice(AccountType::Expense)),
            self.field("interest_account_id","bank-account-interest-account","Interest posts to (income)").kind(InputKind::Select).prompt("Not set").options(choice(AccountType::Income)),
            self.field("match_window_days","bank-account-window","Suggested match window (days)").kind(InputKind::Number).attr("min","0").attr("max","90").hint("How far apart a statement line's and an entry's dates may be for a suggested match."),
            self.field("delimiter","bank-account-delimiter","Column separator").kind(InputKind::Select).options([("Comma",","),("Semicolon",";"),("Tab","\t"),("Pipe","|")].into_iter().map(|(l,v)|Choice::new(l,v)).collect()),
            self.field("header_rows","bank-account-header-rows","Rows before the data").kind(InputKind::Number).attr("min","0").attr("max","20").hint("The last of them is the header row; 0 when the file has none."),
            self.field("date","bank-account-date","Date column").required(),
            self.field("date_format","bank-account-date-format","Date format").kind(InputKind::Select).options(crate::DATE_FORMATS.into_iter().map(|v|Choice::new(v,v)).collect()),
            self.field("description","bank-account-description","Description column").required(),
            self.field("reference","bank-account-reference","Reference column").hint("Optional."),
            self.field("amount","bank-account-amount","Amount column").hint("One signed column; leave blank when the file has debit and credit columns."),
            self.field("amount_sign","bank-account-amount-sign","Sign of the amount column").kind(InputKind::Select).options(vec![Choice::new("Money in is positive","deposits_positive"),Choice::new("Money out is positive","withdrawals_positive")]),
            self.field("debit","bank-account-debit","Debit column (money out)"),
            self.field("credit","bank-account-credit","Credit column (money in)"),
            self.field("balance","bank-account-balance","Running balance column").hint("Optional; with it, each import checks the balances follow from the amounts."),
            self.field("decimal_separator","bank-account-decimal-separator","Decimal separator").kind(InputKind::Select).options(vec![Choice::new("Point (1,234.56)","."),Choice::new("Comma (1.234,56)",",")]),
        ]);
        if self.editing.is_some() {
            fields.push(
                self.field("reason", "bank-account-reason", "Reason for the change")
                    .attr("maxlength", "500"),
            );
        }
        fields
    }
    async fn body(&self, scope: &Scope) -> Result<String> {
        let company = company(scope)?;
        let accounts = told(
            self.pages
                .reconciliation
                .list_bank_accounts(scope, company)
                .await,
        )?;
        let chart = match self.pages.ledger.list_accounts(scope, company).await {
            Ok(v) => Some(v),
            Err(LedgerError::Failed(e)) => return Err(e),
            Err(_) => None,
        };
        let manage = self.pages.allowed(scope, MANAGE_BANK_ACCOUNTS).await?;
        let label = |id: Option<i64>| {
            id.map(|id| {
                chart
                    .as_ref()
                    .and_then(|v| v.iter().find(|a| a.id == id))
                    .map(|a| format!("{} {}", a.code, a.name))
                    .unwrap_or_else(|| format!("#{id}"))
            })
            .unwrap_or_else(|| "—".into())
        };
        let table = if let Ok(accounts) = &accounts {
            let mut rows = vec![];
            for a in accounts {
                let key = a.id.to_string();
                let href = format!("/finance/reconciliation/accounts/{}", a.id);
                let link = render(bilibi_ui::RecordLink {
                    workspace: false,
                    kind: "finance/reconciliation/bank_account",
                    record_id: &key,
                    href: &href,
                    class: "font-medium text-ink-strong hover:underline",
                    body: Html::text(&a.name),
                })?;
                let mut row = FlexRow::new(key)
                    .cell("name", Cell::text(&a.name))
                    .slot("name", link)
                    .cell("ledger", Cell::text(label(Some(a.ledger_account_id))))
                    .cell("charges", Cell::text(label(a.charges_account_id)))
                    .cell("interest", Cell::text(label(a.interest_account_id)))
                    .cell(
                        "window",
                        Cell::text(format!("{} days", a.match_window_days)),
                    )
                    .slot(
                        "status",
                        render(Badge {
                            kind: if a.active {
                                BadgeKind::Success
                            } else {
                                BadgeKind::Neutral
                            },
                            dot: false,
                            class: "",
                            body: Html::text(if a.active { "active" } else { "inactive" }),
                        })?,
                    );
                row.dom_id = Some(format!("bank-account-{}", a.id));
                row.actions = render(Actions {
                    account: a,
                    manage,
                    edit_id: format!("bank-account-{}-edit", a.id),
                    toggle_id: format!("bank-account-{}-toggle", a.id),
                })?;
                rows.push(row);
            }
            let mut table =
                FlexTable::new("bank-accounts-table", self.columns.flex_columns(), rows);
            table.event = "accounts_grid";
            table.actions = true;
            table.framed = false;
            table.zoom = self.columns.zoom;
            table.suggestions = self.columns.flex_suggestions();
            table.add_query = &self.columns.add_query;
            table.empty = Some(TableEmpty::State(EmptyState::missing(
                "No bank accounts yet",
                "Add the first bank account below to start importing its statements.",
            )));
            Some(render(table)?)
        } else {
            None
        };
        View {
            flash: flash_map(&self.flash),
            table,
            forbidden: render(
                EmptyState::forbidden("view bank reconciliation").id("bank-accounts-forbidden"),
            )?,
            chart_forbidden: if accounts.is_ok() && chart.is_none() {
                render(
                    EmptyState::forbidden(
                        "view the ledger's chart of accounts, which a bank account is mapped to",
                    )
                    .id("bank-accounts-chart-forbidden"),
                )?
            } else {
                Html::EMPTY
            },
            manage_forbidden: if accounts.is_ok() && chart.is_some() && !manage {
                render(
                    EmptyState::forbidden("add or change bank accounts")
                        .id("bank-accounts-manage-forbidden"),
                )?
            } else {
                Html::EMPTY
            },
            fields: if accounts.is_ok() && manage {
                chart.as_ref().map(|c| self.fields(c))
            } else {
                None
            },
            editing: self.editing.is_some(),
            heading: self
                .editing
                .as_ref()
                .map(|a| format!("Change {}", a.name))
                .unwrap_or_else(|| "Add a bank account".into()),
            ledger_fixed: self
                .editing
                .as_ref()
                .map(|a| format!("{} (never changes)", label(Some(a.ledger_account_id))))
                .unwrap_or_default(),
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    async fn save(&mut self, scope: &Scope) -> Result<()> {
        if !self.pages.allowed(scope, MANAGE_BANK_ACCOUNTS).await? {
            self.flash = Some((StatusKind::Error, DENIED.into()));
            return Ok(());
        }
        let mapping = MAPPING
            .into_iter()
            .map(|key| {
                let text = self.get(key);
                let text = text.trim();
                let value = if key == "header_rows" {
                    text.parse::<i64>()
                        .ok()
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null)
                } else if [
                    "date",
                    "description",
                    "reference",
                    "amount",
                    "debit",
                    "credit",
                    "balance",
                ]
                .contains(&key)
                {
                    if text.is_empty() {
                        serde_json::Value::Null
                    } else {
                        text.parse::<u32>()
                            .ok()
                            .filter(|v| *v > 0)
                            .map(serde_json::Value::from)
                            .unwrap_or_else(|| text.into())
                    }
                } else if key == "delimiter" {
                    self.get(key).into()
                } else {
                    text.into()
                };
                (key.to_owned(), value)
            })
            .collect::<serde_json::Map<_, _>>();
        let mut errors = vec![];
        let mut optional = |key: &'static str| {
            let text = self.get(key);
            if text.trim().is_empty() {
                None
            } else {
                match text.trim().parse::<i64>() {
                    Ok(id) if id > 0 => Some(id),
                    _ => {
                        errors.push(FieldError::new(key, "is invalid"));
                        None
                    }
                }
            }
        };
        let charges = optional("charges_account_id");
        let interest = optional("interest_account_id");
        if !errors.is_empty() {
            self.errors = errors;
            self.flash = Some((StatusKind::Error, "Refused: account is invalid.".into()));
            return Ok(());
        }
        let result = if let Some(a) = &self.editing {
            self.pages
                .reconciliation
                .update_bank_account(
                    scope,
                    company(scope)?,
                    a.id,
                    &BankAccountEdit {
                        name: Some(self.get("name")),
                        charges_account_id: Some(charges),
                        interest_account_id: Some(interest),
                        match_window_days: Some(
                            self.get("match_window_days").parse().unwrap_or(-1),
                        ),
                        import_mapping: Some(mapping.into()),
                        reason: Some(self.get("reason")),
                        ..Default::default()
                    },
                )
                .await
        } else {
            self.pages
                .reconciliation
                .create_bank_account(
                    scope,
                    company(scope)?,
                    &NewBankAccount {
                        name: self.get("name"),
                        ledger_account_id: self.get("ledger_account_id").parse().unwrap_or(0),
                        charges_account_id: charges,
                        interest_account_id: interest,
                        match_window_days: Some(
                            self.get("match_window_days").parse().unwrap_or(-1),
                        ),
                        import_mapping: mapping.into(),
                        active: None,
                        reason: Some(self.get("reason")),
                    },
                )
                .await
        };
        self.errors = match &result {
            Err(ReconciliationError::Invalid(e)) => e.clone(),
            _ => vec![],
        };
        match told(result)? {
            Ok(a) => {
                self.flash = Some((
                    StatusKind::Success,
                    format!(
                        "Bank account {} {}.",
                        a.name,
                        if self.editing.is_some() {
                            "changed"
                        } else {
                            "added"
                        }
                    ),
                ));
                self.editing = None;
                self.draft = blank();
            }
            Err(e) => self.flash = Some((StatusKind::Error, e)),
        }
        Ok(())
    }
    async fn event(&mut self, scope: &Scope, value: &serde_json::Value) -> Result<()> {
        match event_type(value)? {
            "accounts_grid" => {
                self.columns.handle(&params::string_values(value));
            }
            "change" => self.draft = params::form_values(value, "bank_account"),
            "save_account" => {
                self.draft = params::form_values(value, "bank_account");
                self.save(scope).await?;
            }
            "cancel" => {
                self.editing = None;
                self.errors.clear();
                self.draft = blank();
            }
            "edit" => {
                if !self.pages.allowed(scope, MANAGE_BANK_ACCOUNTS).await? {
                    self.flash = Some((StatusKind::Error, DENIED.into()));
                    return Ok(());
                }
                if let Some(id) = record_id(value) {
                    match told(
                        self.pages
                            .reconciliation
                            .get_bank_account(scope, company(scope)?, id)
                            .await,
                    )? {
                        Ok(a) => {
                            self.draft = blank();
                            let map = serde_json::to_value(&a.import_mapping)
                                .map_err(|_| Error::Internal)?;
                            for key in MAPPING {
                                self.draft.insert(
                                    key.into(),
                                    match &map[key] {
                                        serde_json::Value::Null => String::new(),
                                        serde_json::Value::String(s) => s.clone(),
                                        v => v.to_string(),
                                    },
                                );
                            }
                            for (key, text) in [
                                ("name", a.name.clone()),
                                ("ledger_account_id", a.ledger_account_id.to_string()),
                                (
                                    "charges_account_id",
                                    a.charges_account_id
                                        .map(|id| id.to_string())
                                        .unwrap_or_default(),
                                ),
                                (
                                    "interest_account_id",
                                    a.interest_account_id
                                        .map(|id| id.to_string())
                                        .unwrap_or_default(),
                                ),
                                ("match_window_days", a.match_window_days.to_string()),
                            ] {
                                self.draft.insert(key.into(), text);
                            }
                            self.errors.clear();
                            self.editing = Some(a);
                        }
                        Err(e) => self.flash = Some((StatusKind::Error, e)),
                    }
                }
            }
            "toggle_active" => {
                if !self.pages.allowed(scope, MANAGE_BANK_ACCOUNTS).await? {
                    self.flash = Some((StatusKind::Error, DENIED.into()));
                    return Ok(());
                }
                if let Some(id) = record_id(value) {
                    match told(
                        self.pages
                            .reconciliation
                            .get_bank_account(scope, company(scope)?, id)
                            .await,
                    )? {
                        Ok(a) => match told(
                            self.pages
                                .reconciliation
                                .update_bank_account(
                                    scope,
                                    company(scope)?,
                                    a.id,
                                    &BankAccountEdit {
                                        active: Some(!a.active),
                                        ..Default::default()
                                    },
                                )
                                .await,
                        )? {
                            Ok(a) => {
                                self.flash = Some((
                                    StatusKind::Success,
                                    format!(
                                        "{} is now {}.",
                                        a.name,
                                        if a.active { "active" } else { "inactive" }
                                    ),
                                ))
                            }
                            Err(e) => self.flash = Some((StatusKind::Error, e)),
                        },
                        Err(e) => self.flash = Some((StatusKind::Error, e)),
                    }
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
}
impl LiveState<Scope, Error> for Index {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.flash = None;
            match request.input {
                LiveInput::Refresh => {}
                LiveInput::Params(v) => {
                    self.list = ListState::parse(v, SPEC.clone()).map_err(|_| Error::Invalid)?
                }
                LiveInput::Event(v) => self.event(request.scope, v).await?,
            }
            Ok(LiveBody {
                html: self.body(request.scope).await?,
                query: self.list.to_params(),
                ..Default::default()
            })
        })
    }
}
