//! What receivables needs before anyone can invoice: the accounts new
//! documents post to, the numbering series, and the tax codes. Reads
//! `setup.get`, `setup.changes`, `series.list` and `tax_code.list`; writes call
//! `setup.configure`, `series.create/update` and `tax_code.create/update`.
use super::{
    format::{self as f, Errors},
    render, Account, Flash, Pages, Written, DENIED,
};
use crate::{Series, Setup, SetupChange, TaxCode, MANAGE_SETUP};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Button, ButtonVariant, Card, Cell, Choice, CommitStates, CommitStatus, EmptyState,
    Facts, Field, FlexRow, Form, Header, Html, InlineChoice, InputKind, ListColumns, ModalWidth,
    PageVariant, SectionHeading, StatusKind,
};
use serde_json::json;
use std::collections::HashMap;

enum Dialog {
    Series {
        id: Option<i64>,
        values: HashMap<String, String>,
        errors: Errors,
    },
    Tax {
        id: Option<i64>,
        values: HashMap<String, String>,
        errors: Errors,
    },
}

pub(super) struct Settings {
    pages: Pages,
    setup: Option<Setup>,
    changes: Vec<SetupChange>,
    series: Vec<Series>,
    taxes: Vec<TaxCode>,
    accounts: Vec<Account>,
    can_manage: bool,
    statuses: CommitStates,
    flash: Flash,
    dialog: Option<Dialog>,
    series_cols: ListColumns,
    tax_cols: ListColumns,
    change_cols: ListColumns,
}

impl Settings {
    pub(super) fn new(pages: Pages) -> Result<Self> {
        Ok(Self {
            pages,
            setup: None,
            changes: vec![],
            series: vec![],
            taxes: vec![],
            accounts: vec![],
            can_manage: false,
            statuses: CommitStates::default(),
            flash: Flash::default(),
            dialog: None,
            series_cols: f::columns(
                "receivables-series-table",
                &[
                    ("code", "Code"),
                    ("name", "Name"),
                    ("kind", "Used for"),
                    ("prefix", "Prefix"),
                    ("width", "Digits"),
                    ("status", "Status"),
                    ("actions", ""),
                ],
                &["width"],
            )?,
            tax_cols: f::columns(
                "receivables-tax-table",
                &[
                    ("code", "Code"),
                    ("name", "Name"),
                    ("rate", "Rate"),
                    ("account", "Tax account"),
                    ("status", "Status"),
                    ("actions", ""),
                ],
                &["rate"],
            )?,
            change_cols: f::columns(
                "receivables-setup-changes-table",
                &[
                    ("when", "When"),
                    ("field", "Account"),
                    ("from", "From"),
                    ("to", "To"),
                    ("by", "By"),
                ],
                &[],
            )?,
        })
    }

    pub(super) async fn first(&mut self, scope: &Scope) -> Result<String> {
        self.load(scope).await?;
        self.body(scope).await
    }

    async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.setup = self
            .pages
            .read(scope, "finance.receivables.setup.get", json!({}))
            .await?
            .get("setup")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        self.changes = self
            .pages
            .read_all(
                scope,
                "finance.receivables.setup.changes",
                json!({}),
                ("after", "next_after"),
            )
            .await?
            .0;
        self.series = self
            .pages
            .read_all(
                scope,
                "finance.receivables.series.list",
                json!({}),
                ("after_code", "next_after_code"),
            )
            .await?
            .0;
        self.taxes = self
            .pages
            .read_all(
                scope,
                "finance.receivables.tax_code.list",
                json!({}),
                ("after_code", "next_after_code"),
            )
            .await?
            .0;
        self.accounts = self.pages.accounts(scope).await;
        self.can_manage = self.pages.allowed(scope, MANAGE_SETUP).await?;
        self.series_cols.load(&self.series, |s| s.id.to_string());
        self.tax_cols.load(&self.taxes, |t| t.id.to_string());
        self.change_cols.load(&self.changes, |c| c.id.to_string());
        Ok(())
    }

    fn account_choices(&self, kind: &str) -> Vec<Choice> {
        self.accounts
            .iter()
            .filter(|a| a.active && a.account_type == kind)
            .map(|a| Choice::new(a.label(), a.id.to_string()))
            .collect()
    }

    /// A default account: read first, a choice in place once the company has them.
    fn default_fact(
        &self,
        key: &str,
        label: &'static str,
        current: i64,
        kind: &str,
    ) -> Result<bilibi_ui::Fact<'static>> {
        let mut field = f::inline(
            key,
            label,
            &current.to_string(),
            0,
            "configure",
            self.can_manage,
            &self.statuses,
        );
        field.allow_empty = false;
        field.id_value = "setup".into();
        let editor = Field::named(key)
            .id(format!("receivables-{key}-editor"))
            .label_class("sr-only")
            .label(label)
            .kind(InputKind::Select)
            .value(current.to_string())
            .options(self.account_choices(kind));
        Ok(f::fact_html(
            label,
            render(InlineChoice {
                field,
                display: Html::text(&super::account_label(&self.accounts, current)),
                editor: render(editor)?,
            })?,
        ))
    }

    async fn body(&mut self, scope: &Scope) -> Result<String> {
        let display = self.pages.display(scope).await?;
        let mut body = render(Header {
            title: Html::text("Receivables settings"),
            subtitle: "The accounts, numbering and tax codes invoices and receipts use.",
            ..Header::default()
        })?;

        // Default accounts.
        let mut defaults = render(SectionHeading {
            title: "Default accounts",
            description: "New invoices post to these. An invoice already issued keeps the receivable account it was issued with.",
            ..SectionHeading::default()
        })?;
        match &self.setup {
            Some(setup) => defaults.push(&render(Facts {
                id: "receivables-defaults",
                items: vec![
                    self.default_fact(
                        "receivable_account_id",
                        "Receivable account",
                        setup.receivable_account_id,
                        "asset",
                    )?,
                    self.default_fact(
                        "income_account_id",
                        "Default income account",
                        setup.income_account_id,
                        "income",
                    )?,
                ],
            })?),
            None => {
                let mut form = render(
                    Field::in_form("setup", "receivable_account_id")
                        .id("receivables-setup-receivable")
                        .label("Receivable account")
                        .kind(InputKind::Select)
                        .prompt("Choose an asset account")
                        .options(self.account_choices("asset"))
                        .required()
                        .hint("Money customers owe you is kept here."),
                )?;
                form.push(&render(
                    Field::in_form("setup", "income_account_id")
                        .id("receivables-setup-income")
                        .label("Default income account")
                        .kind(InputKind::Select)
                        .prompt("Choose an income account")
                        .options(self.account_choices("income"))
                        .required()
                        .hint("Used for an invoice line that names no account."),
                )?);
                if self.can_manage {
                    defaults.push(&render(Form {
                        id: "receivables-setup-form",
                        event: "configure",
                        change: false,
                        class: "",
                        body: form,
                        actions: render(Button {
                            id: "receivables-setup-save",
                            button_type: "submit",
                            variant: ButtonVariant::Primary,
                            working: "Saving…",
                            body: Html::text("Save accounts"),
                            ..Button::default()
                        })?,
                    })?);
                } else {
                    defaults.push(&Html::text("Receivables is not set up yet. Someone who manages its setup must choose the default accounts."));
                }
            }
        }
        body.push(&render(Card {
            id: "receivables-defaults-card",
            role: "region",
            body: defaults,
            ..Card::default()
        })?);

        // Series.
        let mut rows = Vec::new();
        for s in &self.series {
            let mut row = FlexRow::new(s.id.to_string());
            row.dom_id = Some(format!("receivables-series-{}", s.id));
            row = row
                .cell("code", Cell::text(&s.code))
                .cell("name", Cell::text(&s.name))
                .cell("kind", Cell::text(f::series_kind_label(&s.kind)))
                .cell("prefix", Cell::text(&s.prefix))
                .cell("width", Cell::text(s.number_width.to_string()))
                .slot("status", f::active_badge(s.active)?);
            if self.can_manage {
                row = row.slot("actions", edit_button("series", s.id)?);
            }
            rows.push(row);
        }
        let mut series = render(SectionHeading {
            title: "Numbering series",
            class: "p-5 pb-0",
            description: "A document is numbered from its series when it is issued: the prefix, then the next number padded to the digits. Code, kind, prefix and digits cannot change once made.",
            actions: if self.can_manage {
                render(Button {
                    id: "receivables-series-add",
                    button_type: "button",
                    event: "show_series",
                    body: Html::text("Add series"),
                    ..Button::default()
                })?
            } else {
                Html::EMPTY
            },
            ..SectionHeading::default()
        })?;
        series.push(&f::table(
            &self.series_cols,
            "series_grid",
            "Numbering series",
            rows,
            "No series yet. Add one each for invoices, credit notes and receipts.",
        )?);
        body.push(&render(Card {
            id: "receivables-series-card",
            role: "region",
            class: "mt-4",
            inner_class: "p-0",
            body: series,
            ..Card::default()
        })?);

        // Tax codes.
        let mut rows = Vec::new();
        for t in &self.taxes {
            let mut row = FlexRow::new(t.id.to_string());
            row.dom_id = Some(format!("receivables-tax-code-{}", t.id));
            row = row
                .cell("code", Cell::text(&t.code))
                .cell("name", Cell::text(&t.name))
                .slot("rate", f::tabular(&format!("{}%", f::decimal(t.rate)))?)
                .cell(
                    "account",
                    Cell::text(super::account_label(&self.accounts, t.account_id)),
                )
                .slot("status", f::active_badge(t.active)?);
            if self.can_manage {
                row = row.slot("actions", edit_button("tax", t.id)?);
            }
            rows.push(row);
        }
        let mut taxes = render(SectionHeading {
            title: "Tax codes",
            class: "p-5 pb-0",
            description: "Tax collected on a line goes to the code's account. A code's rate cannot change; make a new code instead. Documents already saved keep the rate they were saved with.",
            actions: if self.can_manage {
                render(Button {
                    id: "receivables-tax-add",
                    button_type: "button",
                    event: "show_tax",
                    body: Html::text("Add tax code"),
                    ..Button::default()
                })?
            } else {
                Html::EMPTY
            },
            ..SectionHeading::default()
        })?;
        taxes.push(&f::table(
            &self.tax_cols,
            "tax_grid",
            "Tax codes",
            rows,
            "No tax codes. Lines are invoiced without tax until you add one.",
        )?);
        body.push(&render(Card {
            id: "receivables-tax-card",
            role: "region",
            class: "mt-4",
            inner_class: "p-0",
            body: taxes,
            ..Card::default()
        })?);

        // Changes to the defaults.
        let mut rows = Vec::new();
        for c in self.changes.iter().rev() {
            let mut row = FlexRow::new(c.id.to_string());
            row.dom_id = Some(format!("receivables-setup-change-{}", c.id));
            rows.push(
                row.slot(
                    "when",
                    f::time(
                        &display,
                        &format!("receivables-setup-change-at-{}", c.id),
                        &c.changed_at,
                    )?,
                )
                .cell(
                    "field",
                    Cell::text(if c.field == "income_account_id" {
                        "Default income account"
                    } else {
                        "Receivable account"
                    }),
                )
                .cell(
                    "from",
                    Cell::text(
                        c.old_value.map_or_else(
                            || "—".into(),
                            |id| super::account_label(&self.accounts, id),
                        ),
                    ),
                )
                .cell(
                    "to",
                    Cell::text(super::account_label(&self.accounts, c.new_value)),
                )
                .cell(
                    "by",
                    Cell::text(f::actor(
                        Some(c.changed_by_user_id),
                        c.changed_by_impersonator_id,
                    )),
                ),
            );
        }
        let mut history = render(SectionHeading {
            title: "Changes to the default accounts",
            class: "p-5 pb-0",
            ..SectionHeading::default()
        })?;
        history.push(&f::table(
            &self.change_cols,
            "changes_grid",
            "Changes to the default accounts",
            rows,
            "The default accounts have not been changed.",
        )?);
        body.push(&render(Card {
            id: "receivables-changes-card",
            role: "region",
            class: "mt-4",
            inner_class: "p-0",
            body: history,
            ..Card::default()
        })?);

        let modal = self.dialog_html()?;
        if self.setup.is_none() && !self.can_manage {
            let mut empty =
                EmptyState::missing("Not set up", "Receivables has no default accounts yet.");
            empty.id = "receivables-not-set-up".into();
            let _ = empty;
        }
        f::frame(
            "receivables-settings",
            PageVariant::Detail,
            &self.flash,
            body,
            modal,
        )
    }

    fn dialog_html(&self) -> Result<Option<Html>> {
        let Some(dialog) = &self.dialog else {
            return Ok(None);
        };
        let (title, kind, id, values, errors) = match dialog {
            Dialog::Series { id, values, errors } => (
                if id.is_some() {
                    "Change numbering series"
                } else {
                    "Add numbering series"
                },
                "series",
                id,
                values,
                errors,
            ),
            Dialog::Tax { id, values, errors } => (
                if id.is_some() {
                    "Change tax code"
                } else {
                    "Add tax code"
                },
                "tax",
                id,
                values,
                errors,
            ),
        };
        let value = |key: &str| values.get(key).cloned().unwrap_or_default();
        let creating = id.is_none();
        let specs = std::cell::RefCell::new(Vec::<(String, String)>::new());
        let mut body = Html::EMPTY;
        let field = |key: &str, label: &str| -> Field {
            specs.borrow_mut().push((key.to_owned(), label.to_owned()));
            Field::in_form(kind, key)
                .id(format!("receivables-{kind}-{key}"))
                .label(label)
                .value(value(key))
                .errors(errors.of(key))
        };
        if kind == "series" {
            if creating {
                body.push(&render(
                    field("code", "Code").attr("maxlength", "50").required(),
                )?);
            }
            body.push(&render(
                field("name", "Name").attr("maxlength", "200").required(),
            )?);
            if creating {
                body.push(&render(
                    field("kind", "Used for")
                        .kind(InputKind::Select)
                        .options(vec![
                            Choice::new("Invoices", "invoice"),
                            Choice::new("Credit notes", "credit_note"),
                            Choice::new("Receipts", "payment"),
                        ])
                        .prompt("Choose what it numbers")
                        .required(),
                )?);
                body.push(&render(
                    field("prefix", "Prefix")
                        .attr("maxlength", "20")
                        .hint("Printed before the number, such as INV-."),
                )?);
                body.push(&render(
                    field("number_width", "Digits")
                        .kind(InputKind::Number)
                        .attr("min", "1")
                        .attr("max", "12")
                        .hint("How many digits the number is padded to. Empty means 1."),
                )?);
            }
        } else {
            if creating {
                body.push(&render(
                    field("code", "Code").attr("maxlength", "50").required(),
                )?);
            }
            body.push(&render(
                field("name", "Name").attr("maxlength", "200").required(),
            )?);
            if creating {
                body.push(&render(
                    field("rate", "Rate (percent)")
                        .attr("inputmode", "decimal")
                        .required()
                        .hint("For example 6 for six percent. It cannot be changed later."),
                )?);
            }
            body.push(&render(
                field("account_id", "Tax account")
                    .kind(InputKind::Select)
                    .prompt("Choose a liability account")
                    .options(self.account_choices("liability"))
                    .required()
                    .hint("Tax collected is owed to the tax authority, so this is a liability account."),
            )?);
        }
        body.push(&render(
            Field::in_form(kind, "active")
                .id(format!("receivables-{kind}-active"))
                .label("Active")
                .kind(InputKind::Checkbox)
                .checked(values.get("active").is_none_or(|v| v == "true"))
                .hint("An inactive one cannot be chosen for new documents."),
        )?);
        let labels: Vec<_> = specs
            .borrow()
            .iter()
            .map(|(k, l)| (k.clone(), format!("receivables-{kind}-{k}"), l.clone()))
            .collect();
        let mut inner = render(errors.summary(&format!("receivables-{kind}-errors"), &labels))?;
        inner.push(&body);
        let mut actions = render(Button {
            id: &format!("receivables-{kind}-save"),
            button_type: "submit",
            variant: ButtonVariant::Primary,
            working: "Saving…",
            body: Html::text("Save"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: &format!("receivables-{kind}-cancel"),
            button_type: "button",
            event: "cancel_dialog",
            body: Html::text("Cancel"),
            ..Button::default()
        })?);
        let form = render(Form {
            id: &format!("receivables-{kind}-form"),
            event: if kind == "series" {
                "save_series"
            } else {
                "save_tax"
            },
            change: false,
            class: "",
            body: inner,
            actions,
        })?;
        Ok(Some(f::modal(
            &format!("receivables-{kind}"),
            title,
            "",
            "cancel_dialog",
            ModalWidth::Narrow,
            &self.flash,
            form,
        )?))
    }

    async fn save_default(
        &mut self,
        scope: &Scope,
        credential: &str,
        v: &serde_json::Value,
    ) -> Result<()> {
        let key = v["field"].as_str().ok_or(Error::Invalid)?;
        let label = match key {
            "receivable_account_id" => "Receivable account",
            "income_account_id" => "Default income account",
            _ => return Err(Error::Invalid),
        };
        let submitted = v["value"].as_str().ok_or(Error::Invalid)?.trim().to_owned();
        let Ok(id) = submitted.parse::<i64>() else {
            self.statuses.put(
                key,
                CommitStatus::refusal(label, &submitted, &["must be an account"]),
            );
            return Ok(());
        };
        if !self.can_manage {
            self.statuses.put(
                key,
                CommitStatus::Error("You do not have permission to change the setup.".into()),
            );
            return Ok(());
        }
        let written = self
            .pages
            .write(
                scope,
                credential,
                "configure",
                "finance.receivables.setup.configure",
                json!({key: id}),
            )
            .await?;
        self.statuses.put(
            key,
            f::commit_status(
                &written,
                key,
                label,
                &super::account_label(&self.accounts, id),
            ),
        );
        Ok(())
    }

    async fn event(
        &mut self,
        scope: &Scope,
        credential: &str,
        v: &serde_json::Value,
    ) -> Result<()> {
        let id_of = |v: &serde_json::Value| v["id"].as_str().and_then(|i| i.parse::<i64>().ok());
        match v["type"].as_str().ok_or(Error::Invalid)? {
            "clear-flash" => self.flash.dismiss(v),
            "series_grid" => {
                self.series_cols.handle(&params::string_values(v));
            }
            "tax_grid" => {
                self.tax_cols.handle(&params::string_values(v));
            }
            "changes_grid" => {
                self.change_cols.handle(&params::string_values(v));
            }
            "configure" if v["field"].is_string() => {
                self.save_default(scope, credential, v).await?
            }
            "configure" => {
                let form = params::form_values(v, "setup");
                let mut input = json!({});
                for key in ["receivable_account_id", "income_account_id"] {
                    if let Ok(id) = f::posted(&form, key).parse::<i64>() {
                        input[key] = json!(id);
                    }
                }
                match self
                    .pages
                    .write(
                        scope,
                        credential,
                        "configure",
                        "finance.receivables.setup.configure",
                        input,
                    )
                    .await?
                {
                    Written::Done(_) => self
                        .flash
                        .set(StatusKind::Success, "Default accounts saved."),
                    Written::Invalid(fields) => {
                        let message = f::sentences(&fields);
                        self.flash.set(
                            StatusKind::Error,
                            if message.is_empty() {
                                "Choose both accounts.".into()
                            } else {
                                message
                            },
                        );
                    }
                    Written::Refused(message) => self.flash.set(
                        StatusKind::Error,
                        super::worded(
                            message,
                            "You do not have permission to change the receivables setup.",
                        ),
                    ),
                }
            }
            "show_series" | "edit_series" | "show_tax" | "edit_tax" => {
                if !self.can_manage {
                    self.flash.set(StatusKind::Error, DENIED);
                } else {
                    let kind = v["type"].as_str().unwrap_or("");
                    let id = if kind.starts_with("edit") {
                        id_of(v)
                    } else {
                        None
                    };
                    let mut values = HashMap::new();
                    values.insert("active".into(), "true".to_owned());
                    if let Some(id) = id {
                        if kind == "edit_series" {
                            if let Some(s) = self.series.iter().find(|s| s.id == id) {
                                values.insert("name".into(), s.name.clone());
                                values.insert("active".into(), s.active.to_string());
                            }
                        } else if let Some(t) = self.taxes.iter().find(|t| t.id == id) {
                            values.insert("name".into(), t.name.clone());
                            values.insert("account_id".into(), t.account_id.to_string());
                            values.insert("active".into(), t.active.to_string());
                        }
                    }
                    self.dialog = Some(if kind.ends_with("series") {
                        Dialog::Series {
                            id,
                            values,
                            errors: Errors::default(),
                        }
                    } else {
                        Dialog::Tax {
                            id,
                            values,
                            errors: Errors::default(),
                        }
                    });
                }
            }
            "cancel_dialog" => self.dialog = None,
            "save_series" | "save_tax" => {
                let series = v["type"] == "save_series";
                let form = params::form_values(v, if series { "series" } else { "tax" });
                let (id, keys, create, update): (Option<i64>, &[&str], _, _) = match &self.dialog {
                    Some(Dialog::Series { id, .. }) if series => (
                        *id,
                        &["code", "name", "kind", "prefix"][..],
                        "finance.receivables.series.create",
                        "finance.receivables.series.update",
                    ),
                    Some(Dialog::Tax { id, .. }) if !series => (
                        *id,
                        &["code", "name", "rate"][..],
                        "finance.receivables.tax_code.create",
                        "finance.receivables.tax_code.update",
                    ),
                    _ => return Err(Error::Invalid),
                };
                let mut input = json!({});
                let mut local = Errors::default();
                for key in keys {
                    let text = if *key == "prefix" {
                        form.get(*key).map_or("", String::as_str)
                    } else {
                        f::posted(&form, key)
                    };
                    if id.is_some() && *key != "name" {
                        continue;
                    }
                    f::put(&mut input, key, text);
                }
                // A prefix is the one text that keeps its spaces.
                input["active"] = json!(form.get("active").is_some_and(|a| a == "true"));
                if series {
                    if id.is_none() {
                        match f::posted(&form, "number_width") {
                            "" => {}
                            w => match w.parse::<i64>() {
                                Ok(n) => input["number_width"] = json!(n),
                                Err(_) => local
                                    .0
                                    .entry("number_width".into())
                                    .or_default()
                                    .push("must be a whole number".into()),
                            },
                        }
                    }
                } else if let Ok(account) = f::posted(&form, "account_id").parse::<i64>() {
                    input["account_id"] = json!(account);
                } else {
                    local
                        .0
                        .entry("account_id".into())
                        .or_default()
                        .push("can't be blank".into());
                }
                if let Some(id) = id {
                    input[if series { "series_id" } else { "tax_code_id" }] = json!(id);
                }
                let restore = |this: &mut Self, errors: Errors, form: HashMap<String, String>| {
                    this.dialog = Some(if series {
                        Dialog::Series {
                            id,
                            values: form,
                            errors,
                        }
                    } else {
                        Dialog::Tax {
                            id,
                            values: form,
                            errors,
                        }
                    });
                };
                if !local.is_empty() {
                    restore(self, local, form);
                } else {
                    let (event, contract) = match (series, id.is_some()) {
                        (true, false) => ("save_series", create),
                        (true, true) => ("save_series", update),
                        (false, false) => ("save_tax", create),
                        (false, true) => ("save_tax", update),
                    };
                    match self
                        .pages
                        .write(scope, credential, event, contract, input)
                        .await?
                    {
                        Written::Done(_) => {
                            self.dialog = None;
                            self.flash.set(
                                StatusKind::Success,
                                if series {
                                    "Numbering series saved."
                                } else {
                                    "Tax code saved."
                                },
                            );
                        }
                        Written::Invalid(fields) => restore(self, Errors(fields), form),
                        Written::Refused(message) => {
                            self.dialog = None;
                            self.flash.set(
                                StatusKind::Error,
                                super::worded(
                                    message,
                                    "You do not have permission to change the receivables setup.",
                                ),
                            );
                        }
                    }
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
}

fn edit_button(kind: &str, id: i64) -> Result<Html> {
    render(Button {
        id: &format!("receivables-{kind}-edit-{id}"),
        button_type: "button",
        event: if kind == "series" {
            "edit_series"
        } else {
            "edit_tax"
        },
        name: "id",
        value: &id.to_string(),
        body: Html::text("Edit"),
        ..Button::default()
    })
}

impl LiveState<Scope, Error> for Settings {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(v) = r.input {
                self.event(r.scope, r.credential, v).await?;
            }
            self.load(r.scope).await?;
            Ok(LiveBody {
                html: self.body(r.scope).await?,
                ..LiveBody::default()
            })
        })
    }
}
