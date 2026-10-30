//! Vendor settings and explicit historical matches, through the same
//! contracts as agents. Company creation and relationships belong to Core.
use super::{company, render, Pages, Written, PAGES};
use crate::{SupplierHistory, Vendor, MANAGE_SETTINGS};
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::{
    record_company_relationship, ChosenCompany, CompanyErrors, CompanyPickerContext,
    MultiCompanyChooser,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    *,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

const FORM: &str = "vendor";
static SPEC: LazyLock<Arc<ListSpec>> =
    LazyLock::new(|| ListSpec::default().shared().expect("vendor list"));
struct Create {
    chooser: MultiCompanyChooser,
    values: HashMap<String, String>,
    errors: CompanyErrors,
}
pub(super) struct Vendors {
    pages: Pages,
    all: Vec<Vendor>,
    history: Vec<SupplierHistory>,
    list: ListState,
    columns: ListColumns,
    history_columns: ListColumns,
    can_manage: bool,
    create: Option<Create>,
    mapping: Option<(String, HashMap<String, String>, CompanyErrors)>,
    notice: Option<(StatusKind, String)>,
    company_name: String,
    states: CommitStates,
}
impl Vendors {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            pages,
            all: vec![],
            history: vec![],
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Invalid)?,
            columns: ListColumns::mount(
                "payables-vendors-table",
                [
                    ("code", "Code"),
                    ("company", "Company"),
                    ("terms", "Terms (days)"),
                    ("payable", "Payable account"),
                    ("expense", "Default expense"),
                    ("tax", "Default tax"),
                    ("active", "Finance availability"),
                    ("relationship", "Relationship"),
                ]
                .into_iter()
                .map(|(k, l)| BuiltinColumn::new(k, l))
                .collect(),
            )
            .map_err(|_| Error::Internal)?,
            history_columns: ListColumns::mount(
                "payables-vendor-keys",
                [
                    ("key", "Supplier key"),
                    ("facts", "Recorded snapshots"),
                    ("collisions", "Possible collisions"),
                    ("vendor", "Confirmed vendor"),
                ]
                .into_iter()
                .map(|(k, l)| BuiltinColumn::new(k, l))
                .collect(),
            )
            .map_err(|_| Error::Internal)?,
            can_manage: false,
            create: None,
            mapping: None,
            notice: None,
            company_name: String::new(),
            states: CommitStates::default(),
        })
    }
    fn context(&self) -> CompanyPickerContext {
        CompanyPickerContext {
            records: self.pages.records.clone(),
            contracts: self.pages.contracts.clone(),
            service: PAGES,
            role: "vendor",
        }
    }
    pub(super) async fn reload(&mut self, s: &Scope) -> Result<()> {
        self.can_manage = self.pages.allowed(s, MANAGE_SETTINGS).await?;
        let Some(company) = company(s) else {
            self.all.clear();
            self.history.clear();
            return Ok(());
        };
        self.company_name = self.pages.company_name(s).await;
        let mut input = json!({"company_id":company,"page_size":100});
        self.all.clear();
        loop {
            let result = self
                .read(s, "finance.payables.vendor.list", input.clone())
                .await?;
            for v in result["records"].as_array().into_iter().flatten() {
                self.all
                    .push(serde_json::from_value(v.clone()).map_err(|_| Error::Internal)?);
            }
            if result["next_after_id"].is_null() {
                break;
            }
            input["after_id"] = result["next_after_id"].clone();
        }
        let history = self
            .read(
                s,
                "finance.payables.vendor.supplier_keys",
                json!({"company_id":company}),
            )
            .await?;
        self.history =
            serde_json::from_value(history["records"].clone()).map_err(|_| Error::Internal)?;
        self.columns.load(&self.all, |v| v.id.to_string());
        self.history_columns
            .load(&self.history, |h| h.supplier_key.clone());
        Ok(())
    }
    async fn read(&self, s: &Scope, key: &str, input: Value) -> Result<Value> {
        self.pages
            .contracts
            .read(s, key, &input)
            .await
            .map(|mut v| v["result"].take())
            .map_err(|e| match e {
                bilibi_agent_api::Error::Forbidden => Error::Forbidden,
                bilibi_agent_api::Error::Backend(e) => e,
                _ => Error::Internal,
            })
    }
    fn button(id: &str, event: &str, label: &str, name: &str, value: &str) -> Result<Html> {
        render(Button {
            id,
            button_type: "button",
            event,
            name,
            value,
            body: Html::text(label),
            ..Button::default()
        })
    }
    fn inline(&self, v: &Vendor, key: &str, label: &str, value: String) -> InlineField {
        let mut field = InlineField::new(format!("vendor-{}-{key}", v.id), key, label, value)
            .status(self.states.get(&format!("{}-{key}", v.id)));
        field.id_value = v.id.to_string();
        field.save_event = "save_vendor".into();
        field.allow_empty = true;
        if !self.can_manage {
            field = field.readonly();
        }
        field
    }
    pub(super) async fn body(&self, s: &Scope) -> Result<String> {
        let mut actions = Html::EMPTY;
        if self.can_manage {
            actions = Self::button("vendor-add", "show_vendor", "Add vendor", "", "")?;
        }
        let mut body = render(Header {
            title: Html::text("Vendors"),
            subtitle: &format!("Vendor settings for {}", self.company_name),
            title_actions: bilibi_ui::Html::EMPTY,
            actions,
        })?;
        if let Some((kind, message)) = &self.notice {
            body.push(&render(Alert {
                id: "vendors-notice",
                kind: *kind,
                body: Html::text(message),
                ..Alert::default()
            })?);
        }
        let (accounts, taxes) = if let Some(company) = company(s) {
            let mut input = json!({"company_id":company,"page_size":100});
            let mut accounts = Vec::new();
            loop {
                let a = self
                    .read(s, "finance.ledger.account.list", input.clone())
                    .await;
                let a = match a {
                    Ok(a) => a,
                    Err(Error::Forbidden | Error::Unauthorized) => break,
                    Err(e) => return Err(e),
                };
                for row in a["records"].as_array().into_iter().flatten() {
                    if row["active"] == true && row["is_group"] != true {
                        accounts.push(row.clone());
                    }
                }
                if a["next_after_code"].is_null() {
                    break;
                }
                input["after_code"] = a["next_after_code"].clone();
            }
            let mut tax_input = json!({"company_id":company,"page_size":100});
            let mut taxes = Vec::new();
            loop {
                let page = self
                    .read(s, "finance.payables.tax_code.list", tax_input.clone())
                    .await?;
                taxes.extend(page["records"].as_array().into_iter().flatten().cloned());
                if page["next_after_id"].is_null() {
                    break;
                }
                tax_input["after_id"] = page["next_after_id"].clone();
            }
            (accounts, taxes)
        } else {
            (vec![], vec![])
        };
        let mut rows = vec![];
        body.push(&render(FilterToolbar::new(
            "vendor-filters",
            vec![ToolbarControl::search(
                "vendor-search",
                "search",
                "Search vendors",
                &self.list.search,
                "Search vendors",
            )],
        ))?);
        let search = self.list.search.to_lowercase();
        let filtered: Vec<_> = self
            .all
            .iter()
            .filter(|v| {
                v.code.to_lowercase().contains(&search)
                    || v.company.name.to_lowercase().contains(&search)
            })
            .collect();
        let total = filtered.len() as u64;
        let start = ((self.list.page.saturating_sub(1)) * self.list.page_size) as usize;
        for v in filtered
            .into_iter()
            .skip(start)
            .take(self.list.page_size as usize)
        {
            let mut row = FlexRow::new(v.id.to_string()).cell("code", Cell::text(&v.code));
            let mut company_body = render(ActionLink {
                href: &v.company.path,
                body: Html::text(&format!("{} · {}", v.company.name, v.company.status)),
                ..ActionLink::default()
            })?;
            if self.columns.mode() != RowMode::Compact {
                let mut facts = vec![];
                for (key, label, value) in [
                    ("tax_id", "Tax ID", &v.company.tax_id),
                    (
                        "registration_number",
                        "Registration number",
                        &v.company.registration_number,
                    ),
                    ("email", "Email", &v.company.email),
                    ("phone", "Phone", &v.company.phone),
                ] {
                    let value =
                        if !v.company.readable || v.company.restricted.iter().any(|r| r == key) {
                            render(Restricted::new(format!("vendor-{}-{key}-restricted", v.id)))?
                        } else {
                            Html::text(value.as_deref().unwrap_or("—"))
                        };
                    facts.push(Fact {
                        title: label,
                        value,
                        ..Fact::default()
                    });
                }
                let billing = match self
                    .read(
                        s,
                        "core.address.company.list",
                        json!({"company_id":v.company.id}),
                    )
                    .await
                {
                    Ok(rows) => {
                        let lines = bilibi_finance_ledger::company_billing_lines(
                            &rows,
                            &self.pages.today(s).await?.to_string(),
                        );
                        Html::text(&if lines.is_empty() {
                            "No billing address in Companies".into()
                        } else {
                            lines.join(", ")
                        })
                    }
                    Err(Error::Forbidden | Error::Unauthorized) => render(
                        Restricted::new(format!("vendor-{}-billing-restricted", v.id))
                            .reason("Viewing addresses needs permission."),
                    )?,
                    Err(e) => return Err(e),
                };
                facts.push(Fact {
                    title: "Billing address",
                    value: billing,
                    ..Fact::default()
                });
                company_body.push(&render(Fold {
                    id: &format!("vendor-{}-company-details", v.id),
                    title: "Company details",
                    summary: "",
                    open: false,
                    body: render(Facts {
                        id: &format!("vendor-{}-facts", v.id),
                        items: facts,
                    })?,
                })?);
            }
            row = row.slot("company", company_body);
            row = row.slot(
                "terms",
                render(InlineEdit {
                    field: self.inline(
                        v,
                        "payment_terms_days",
                        "Payment terms",
                        v.payment_terms_days.to_string(),
                    ),
                })?,
            );
            for (key, label, current, kind) in [
                (
                    "payable_account_id",
                    "Payable account",
                    v.payable_account_id,
                    "liability",
                ),
                (
                    "expense_account_id",
                    "Default expense",
                    v.expense_account_id,
                    "expense",
                ),
            ] {
                let choices = accounts
                    .iter()
                    .filter(|a| {
                        a["account_type"] == kind
                            || (kind == "expense" && a["account_type"] == "asset")
                    })
                    .map(|a| {
                        Choice::new(a["name"].as_str().unwrap_or(""), a["id"].to_string())
                            .detail(a["code"].as_str().unwrap_or(""))
                    })
                    .collect();
                let display = accounts
                    .iter()
                    .find(|a| a["id"].as_i64() == current)
                    .map(|a| {
                        Html::text(&format!(
                            "{} · {}",
                            a["name"].as_str().unwrap_or(""),
                            a["code"].as_str().unwrap_or("")
                        ))
                    })
                    .unwrap_or_else(|| {
                        Html::text(&current.map_or_else(
                            || {
                                if kind == "liability" {
                                    "Company default"
                                } else {
                                    "No default"
                                }
                                .into()
                            },
                            |id| format!("Account {id}"),
                        ))
                    });
                let editor = render(
                    Combobox::new(format!("vendor-{}-{key}-input", v.id), key)
                        .aria_label(label)
                        .value(current.map(|i| i.to_string()).unwrap_or_default())
                        .options(choices),
                )?;
                row = row.slot(
                    if kind == "liability" {
                        "payable"
                    } else {
                        "expense"
                    },
                    render(InlineChoice {
                        field: self.inline(
                            v,
                            key,
                            label,
                            current.map(|i| i.to_string()).unwrap_or_default(),
                        ),
                        display,
                        editor,
                    })?,
                );
            }
            let options = taxes
                .iter()
                .filter(|t| t["active"] == true)
                .map(|t| {
                    Choice::new(t["name"].as_str().unwrap_or(""), t["id"].to_string())
                        .detail(t["code"].as_str().unwrap_or(""))
                })
                .collect();
            row = row.slot(
                "tax",
                render(InlineChoice {
                    field: self.inline(
                        v,
                        "tax_code_id",
                        "Default tax",
                        v.tax_code_id.map(|id| id.to_string()).unwrap_or_default(),
                    ),
                    display: Html::text(
                        taxes
                            .iter()
                            .find(|t| t["id"].as_i64() == v.tax_code_id)
                            .and_then(|t| t["name"].as_str())
                            .unwrap_or("No default"),
                    ),
                    editor: render(
                        Combobox::new(format!("vendor-{}-tax_code_id-input", v.id), "tax_code_id")
                            .aria_label("Default tax")
                            .value(v.tax_code_id.map(|id| id.to_string()).unwrap_or_default())
                            .options(options),
                    )?,
                })?,
            );
            row = row.slot(
                "active",
                render(InlineChoice {
                    field: self.inline(v, "active", "Finance availability", v.active.to_string()),
                    display: Html::text(if v.active { "Available" } else { "Unavailable" }),
                    editor: render(
                        Field::named("active")
                            .id(format!("vendor-{}-active-input", v.id))
                            .label("Finance availability")
                            .kind(InputKind::Select)
                            .value(v.active.to_string())
                            .options(vec![
                                Choice::new("Available", "true"),
                                Choice::new("Unavailable", "false"),
                            ]),
                    )?,
                })?,
            );
            let relation =
                bilibi_finance_ledger::company_relation(&self.context(), s, v.company.id).await?;
            let relation = match relation {
                bilibi_finance_ledger::CompanyRelation::Recorded { .. } => {
                    Html::text("Vendor relationship recorded")
                }
                bilibi_finance_ledger::CompanyRelation::Hidden => {
                    Html::text("No access to company relationships")
                }
                bilibi_finance_ledger::CompanyRelation::Missing if self.can_manage => Self::button(
                    &format!("vendor-{}-relationship", v.id),
                    "record_vendor_relationship",
                    "Record vendor relationship",
                    "vendor",
                    &v.id.to_string(),
                )?,
                _ => Html::text("No vendor relationship recorded"),
            };
            row = row.slot("relationship", relation);
            rows.push(row);
        }
        let mut table = FlexTable::new("payables-vendors-table", self.columns.flex_columns(), rows);
        table.caption = "Vendors";
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(render(EmptyState::missing(
            "No vendors",
            "Add a linked company before recording a new bill.",
        ))?));
        body.push(&render(table)?);
        body.push(&render(Pagination::new(
            "vendor-pagination",
            PageCounts::new(self.list.page, self.list.page_size, total),
            &[25, 50, 100],
        ))?);
        body.push(&render(SectionHeading{title:"Historical supplier keys",description:"Confirm each match after reviewing every snapshot and possible collision. Confirmations are permanent. Bills, payment keys and aging groups remain unchanged.",..SectionHeading::default()})?);
        let mut rows = vec![];
        for h in &self.history {
            let snapshots = h
                .snapshots
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| {
                    format!(
                        "{} | Tax ID: {} | Registration: {} | Address: {}",
                        v["name"].as_str().unwrap_or("—"),
                        v["tax_id"].as_str().unwrap_or("—"),
                        v["registration_id"].as_str().unwrap_or("—"),
                        v["address"].as_str().unwrap_or("—")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            let matched = match h.vendor_id {
                Some(id) => Html::text(&self.all.iter().find(|v| v.id == id).map_or_else(
                    || format!("Vendor {id}"),
                    |v| format!("{} · {}", v.code, v.company.name),
                )),
                None if self.can_manage => Self::button(
                    &format!("vendor-key-{}", h.supplier_key),
                    "show_mapping",
                    "Review match",
                    "key",
                    &h.supplier_key,
                )?,
                None => Html::text("Unmatched: review required"),
            };
            rows.push(
                FlexRow::new(&h.supplier_key)
                    .cell(
                        "key",
                        Cell::text(format!("{} · {} documents", h.supplier_key, h.documents)),
                    )
                    .cell("facts", Cell::text(snapshots))
                    .cell("collisions", Cell::text(h.collision_keys.join(", ")))
                    .slot("vendor", matched),
            );
        }
        let mut history = FlexTable::new(
            "payables-vendor-keys",
            self.history_columns.flex_columns(),
            rows,
        );
        history.caption = "Historical supplier keys";
        history.event = "keys_grid";
        history.zoom = self.history_columns.zoom;
        body.push(&render(history)?);
        if let Some(create) = &self.create {
            body.push(&self.create_dialog(create)?);
        }
        if let Some((key, values, errors)) = &self.mapping {
            body.push(&self.map_dialog(key, values, errors)?);
        }
        render(bilibi_ui::Page {
            id: "payables-vendors",
            variant: PageVariant::List,
            class: "",
            body,
        })
        .map(|h| h.as_str().to_owned())
    }
    fn modal(&self, id: &str, title: &str, body: Html, event: &str, label: &str) -> Result<Html> {
        let mut actions = Self::button(&format!("{id}-cancel"), "cancel_vendor", "Cancel", "", "")?;
        actions.push(&render(Button {
            id: &format!("{id}-submit"),
            event,
            button_type: "button",
            variant: ButtonVariant::Primary,
            body: Html::text(label),
            ..Button::default()
        })?);
        render(Modal {
            id,
            title,
            cancel_event: "cancel_vendor",
            width: ModalWidth::Wide,
            description: &format!("For {}", self.company_name),
            role: "dialog",
            flash: None,
            body: render(Form {
                id: &format!("{id}-form"),
                event: "vendor_change",
                change: true,
                class: "mt-4",
                body,
                actions: render(Actions { body: actions })?,
            })?,
        })
    }
    fn create_dialog(&self, c: &Create) -> Result<Html> {
        let mut labels = c.chooser.labels();
        labels.push(("code".into(), "vendor-code".into(), "Code".into()));
        labels.push((
            "payment_terms_days".into(),
            "vendor-payment_terms_days".into(),
            "Payment terms".into(),
        ));
        let mut body = render(c.errors.summary("vendor-errors", &labels))?;
        body.push(&c.chooser.render(&c.errors)?);
        for (key, label) in [
            ("code", "Code"),
            ("payment_terms_days", "Payment terms (days)"),
        ] {
            if key == "code" && !c.chooser.is_new() {
                continue;
            }
            let field = Field::in_form(FORM, key)
                .id(format!("vendor-{key}"))
                .label(label)
                .value(c.values.get(key).cloned().unwrap_or_default())
                .errors(c.errors.under(key, label));
            let field = if key == "code" {
                field.required().attr("maxlength", "50")
            } else {
                field
                    .kind(InputKind::Number)
                    .attr("min", "0")
                    .attr("max", "3650")
                    .attr("step", "1")
            };
            body.push(&render(field)?);
        }
        self.modal(
            "vendor-new-modal",
            "Add suppliers",
            body,
            "create_vendor",
            if c.chooser.count() == 1 {
                "Add supplier"
            } else {
                "Add suppliers"
            },
        )
    }
    fn map_dialog(
        &self,
        key: &str,
        values: &HashMap<String, String>,
        errors: &CompanyErrors,
    ) -> Result<Html> {
        let mut body = render(errors.summary(
            "vendor-map-errors",
            &[
                (
                    "vendor_id".into(),
                    "vendor-map-choice".into(),
                    "Vendor".into(),
                ),
                ("reason".into(), "vendor-map-reason".into(), "Reason".into()),
            ],
        ))?;
        body.push(&Html::text(&format!(
            "Supplier key: {key}. Review all recorded facts in the row before confirming."
        )));
        body.push(&render(
            Combobox::new("vendor-map-choice", format!("{FORM}[vendor_id]"))
                .label("Vendor *")
                .aria_label("Vendor")
                .attrs(bilibi_ui::Attrs {
                    required: true,
                    aria_label: Some("Vendor".into()),
                    ..bilibi_ui::Attrs::default()
                })
                .value(values.get("vendor_id").cloned().unwrap_or_default())
                .options(
                    self.all
                        .iter()
                        .map(|v| Choice::new(&v.company.name, v.id.to_string()).detail(&v.code))
                        .collect(),
                )
                .errors(errors.under("vendor_id", "Vendor")),
        )?);
        body.push(&render(
            Field::in_form(FORM, "reason")
                .id("vendor-map-reason")
                .label("Reason for this match")
                .required()
                .value(values.get("reason").cloned().unwrap_or_default())
                .errors(errors.under("reason", "Reason")),
        )?);
        self.modal(
            "vendor-map-modal",
            "Confirm supplier match",
            body,
            "confirm_vendor_mapping",
            "Confirm match",
        )
    }
    async fn take(&mut self, _s: &Scope, v: &Value) -> Result<()> {
        let mut form = params::form_values(v, FORM);
        if let Some(c) = &self.create {
            for row in c.chooser.rows() {
                let key = format!("code_{}", row.id);
                if let Some(value) = v[format!("{FORM}[{key}]")].as_str() {
                    if value.len() > 1024 {
                        return Err(Error::Invalid);
                    }
                    form.insert(key, value.to_owned());
                }
            }
        }
        if let Some(c) = &mut self.create {
            c.values.extend(form.clone());
            c.chooser.read(&form);
        }
        if let Some((_, values, _)) = &mut self.mapping {
            values.extend(form);
        }
        Ok(())
    }
    async fn event(&mut self, s: &Scope, credential: &str, v: &Value) -> Result<LiveBody> {
        let event = v["type"].as_str().ok_or(Error::Invalid)?;
        let context = self.context();
        if matches!(
            event,
            "create_vendor"
                | "save_vendor"
                | "confirm_vendor_mapping"
                | "record_vendor_relationship"
                | "show_mapping"
        ) && !self.pages.allowed(s, MANAGE_SETTINGS).await?
        {
            self.notice = Some((
                StatusKind::Error,
                "You do not have permission to manage vendors.".into(),
            ));
            self.reload(s).await?;
            return Ok(LiveBody {
                html: self.body(s).await?,
                ..LiveBody::default()
            });
        }
        match event {
            "filters" => self.list.apply_filters(&params::form_values(v, "filters")),
            "page" => self.list.put_page(v["page"].as_str()),
            "grid" => {
                self.columns.handle(&params::string_values(v));
            }
            "keys_grid" => {
                self.history_columns.handle(&params::string_values(v));
            }
            "show_vendor" => {
                if self.pages.allowed(s, MANAGE_SETTINGS).await? {
                    self.reload(s).await?;
                    let existing = self
                        .all
                        .iter()
                        .map(|vendor| {
                            let query = url::form_urlencoded::Serializer::new(String::new())
                                .append_pair("search", &vendor.code)
                                .finish();
                            (
                                vendor.record_id.clone(),
                                format!(
                                    "/finance/payables/vendors?{query}#payables-vendors-table-row-{}",
                                    vendor.id
                                ),
                            )
                        })
                        .collect();
                    self.create = Some(Create {
                        chooser: MultiCompanyChooser::new("vendor-new", FORM).existing(
                            "Already a supplier",
                            "View supplier",
                            existing,
                        ),
                        values: HashMap::new(),
                        errors: CompanyErrors::default(),
                    });
                }
            }
            "cancel_vendor" => {
                self.create = None;
                self.mapping = None;
            }
            "vendor_change" => self.take(s, v).await?,
            "search_companies" => {
                if let Some(c) = &mut self.create {
                    c.chooser
                        .search(&context, s, v["query"].as_str().unwrap_or(""))
                        .await?;
                }
            }
            "pick_companies" => {
                if let Some(c) = &mut self.create {
                    c.chooser
                        .pick(&context, s, v["value"].as_str().ok_or(Error::Invalid)?)
                        .await?;
                    c.errors = CompanyErrors::default();
                }
            }
            "remove_company" => {
                if let Some(c) = &mut self.create {
                    c.chooser.remove(v["value"].as_str().ok_or(Error::Invalid)?);
                }
            }
            "new_company" => {
                if let Some(c) = &mut self.create {
                    c.chooser.start_new(v["query"].as_str().unwrap_or(""));
                }
            }
            "cancel_new_company" => {
                if let Some(c) = &mut self.create {
                    c.chooser.cancel_new();
                }
            }
            "use_candidate" => {
                if let Some(c) = &mut self.create {
                    c.chooser
                        .use_candidate(&context, s, v["company"].as_str().ok_or(Error::Invalid)?)
                        .await?;
                    c.errors = CompanyErrors::default();
                }
            }
            "show_mapping" => {
                let key = v["key"].as_str().ok_or(Error::Invalid)?;
                if self
                    .history
                    .iter()
                    .any(|h| h.supplier_key == key && h.vendor_id.is_none())
                {
                    self.mapping = Some((key.into(), HashMap::new(), CompanyErrors::default()));
                }
            }
            "create_vendor" => {
                self.take(s, v).await?;
                if let Some(c) = &mut self.create {
                    c.errors = CompanyErrors::default();
                    let terms = c
                        .values
                        .get("payment_terms_days")
                        .map(String::as_str)
                        .unwrap_or("");
                    let days = if terms.is_empty() {
                        Some(0)
                    } else {
                        terms.parse::<i32>().ok().filter(|n| (0..=3650).contains(n))
                    };
                    let Some(days) = days else {
                        c.errors = CompanyErrors(BTreeMap::from([(
                            "payment_terms_days".into(),
                            vec!["must be between 0 and 3650".into()],
                        )]));
                        return Ok(LiveBody {
                            html: self.body(s).await?,
                            ..LiveBody::default()
                        });
                    };
                    if c.chooser.count() == 0 {
                        c.errors = CompanyErrors(BTreeMap::from([(
                            "base".into(),
                            vec!["Choose a company.".into()],
                        )]));
                    } else {
                        if c.chooser.is_new() {
                            match c
                                .chooser
                                .chooser()
                                .company(&context, s, credential, event)
                                .await?
                            {
                                ChosenCompany::Company(_) => {
                                    c.chooser.capture_created(
                                        c.values.get("code").map(String::as_str).unwrap_or(""),
                                    )?;
                                }
                                ChosenCompany::Invalid(errors) => {
                                    c.errors = errors;
                                }
                                ChosenCompany::Refused(message) => {
                                    c.errors = CompanyErrors(BTreeMap::from([(
                                        "base".into(),
                                        vec![message],
                                    )]));
                                }
                                ChosenCompany::Review => {
                                    c.errors = CompanyErrors::default();
                                }
                            }
                        }
                        let rows: Vec<_> = c
                            .chooser
                            .rows()
                            .iter()
                            .map(|r| (r.id.clone(), r.code.clone()))
                            .collect();
                        let mut added = 0;
                        let mut warnings = Vec::new();
                        for (id, code) in rows {
                            let target = id.parse::<i64>().map_err(|_| Error::Invalid)?;
                            let input = json!({"company_id": company(s), "core_company_id": target, "code": code, "payment_terms_days": days, "active": true});
                            match self.pages.write(s, credential, event, "finance.payables.vendor.create", input).await? {
                                Written::Done(record) => {
                                    added += 1;
                                    if let Some(vendor_id) = super::record_id(&record) {
                                        let query = url::form_urlencoded::Serializer::new(String::new()).append_pair("search", &code).finish();
                                        c.chooser.mark_existing(&id, format!("/finance/payables/vendors?{query}#payables-vendors-table-row-{vendor_id}"));
                                    }
                                    c.chooser.remove(&id);
                                    if let Err(reason) = record_company_relationship(&context, s, credential, event, target).await? { warnings.push(reason); }
                                }
                                Written::Fields(errors) => c.chooser.row_errors(&id, CompanyErrors(errors)),
                                _ => c.chooser.row_errors(&id, CompanyErrors(BTreeMap::from([("base".into(), vec!["Supplier was not added. Check your permission and try again.".into()])]))),
                            }
                        }
                        if added > 0 {
                            self.notice = Some(if warnings.is_empty() {
                                (StatusKind::Success, format!("{added} supplier(s) added."))
                            } else {
                                (
                                    StatusKind::Warning,
                                    format!("{added} supplier(s) added. {}", warnings.join("; ")),
                                )
                            });
                        }
                        if c.chooser.count() == 0 {
                            self.create = None;
                        }
                    }
                }
            }
            "confirm_vendor_mapping" => {
                self.take(s, v).await?;
                if let Some((key, values, errors)) = &mut self.mapping {
                    let input = json!({"company_id":company(s),"vendor_id":values.get("vendor_id").and_then(|v|v.parse::<i64>().ok()),"supplier_key":key,"reason":values.get("reason").cloned().unwrap_or_default()});
                    match self
                        .pages
                        .write(
                            s,
                            credential,
                            event,
                            "finance.payables.vendor.map_supplier_key",
                            input,
                        )
                        .await?
                    {
                        Written::Done(_) => {
                            self.mapping = None;
                            self.notice=Some((StatusKind::Success,"Supplier match confirmed. Historical records and balances are unchanged.".into()));
                        }
                        Written::Fields(e) => *errors = CompanyErrors(e),
                        _ => {
                            *errors = CompanyErrors(BTreeMap::from([(
                                "base".into(),
                                vec![
                                    "Match was not confirmed. Check your permission and try again."
                                        .into(),
                                ],
                            )]))
                        }
                    }
                }
            }
            "save_vendor" => {
                let id = v["id"]
                    .as_str()
                    .and_then(|v| v.parse::<i64>().ok())
                    .ok_or(Error::Invalid)?;
                let key = v["field"].as_str().ok_or(Error::Invalid)?;
                let value = v["value"].as_str().ok_or(Error::Invalid)?;
                if !self.all.iter().any(|v| v.id == id) {
                    return Err(Error::Invalid);
                }
                let mut input = json!({"company_id":company(s),"vendor_id":id});
                if matches!(
                    key,
                    "payable_account_id" | "expense_account_id" | "tax_code_id"
                ) && value.trim().is_empty()
                {
                    input[match key {
                        "payable_account_id" => "clear_payable_account",
                        "expense_account_id" => "clear_expense_account",
                        _ => "clear_tax_code",
                    }] = json!(true);
                } else {
                    input[key] = match key {
                        "payment_terms_days" => {
                            value.parse::<i32>().map(|v| json!(v)).unwrap_or(json!(-1))
                        }
                        "active" => json!(value == "true"),
                        "payable_account_id" | "expense_account_id" | "tax_code_id" => {
                            value.parse::<i64>().map(|v| json!(v)).unwrap_or(json!(0))
                        }
                        _ => return Err(Error::Invalid),
                    };
                }
                let status = match self
                    .pages
                    .write(
                        s,
                        credential,
                        event,
                        "finance.payables.vendor.update",
                        input,
                    )
                    .await?
                {
                    Written::Done(_) => CommitStatus::Saved,
                    Written::Fields(e) => CommitStatus::Error(
                        e.into_values().flatten().collect::<Vec<_>>().join("; "),
                    ),
                    _ => CommitStatus::Error(
                        "The change was not saved. Check your permission and try again.".into(),
                    ),
                };
                self.states.put(format!("{id}-{key}"), status);
            }
            "record_vendor_relationship" => {
                let id = v["vendor"]
                    .as_str()
                    .and_then(|v| v.parse::<i64>().ok())
                    .ok_or(Error::Invalid)?;
                let target = self
                    .all
                    .iter()
                    .find(|v| v.id == id)
                    .ok_or(Error::Invalid)?
                    .company
                    .id;
                let recorded =
                    record_company_relationship(&context, s, credential, event, target).await?;
                self.notice = Some(match recorded {
                    Ok(()) => (
                        StatusKind::Success,
                        "Vendor relationship recorded in Companies.".into(),
                    ),
                    Err(e) => (StatusKind::Warning, e),
                });
            }
            _ => return Err(Error::Invalid),
        }
        self.reload(s).await?;
        Ok(LiveBody {
            html: self.body(s).await?,
            query: self.list.to_params(),
            ..LiveBody::default()
        })
    }
}
impl LiveState<Scope, Error> for Vendors {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Event(v) => return self.event(r.scope, r.credential, v).await,
                LiveInput::Refresh | LiveInput::Params(_) => self.reload(r.scope).await?,
            }
            Ok(LiveBody {
                html: self.body(r.scope).await?,
                ..LiveBody::default()
            })
        })
    }
}
#[derive(askama::Template)]
#[template(
    source = r#"<div class="flex w-full justify-end gap-2">{{ body|safe }}</div>"#,
    ext = "html"
)]
struct Actions {
    body: Html,
}
