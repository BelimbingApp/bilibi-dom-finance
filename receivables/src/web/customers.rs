//! The customers list and its Add customer dialog. Reads
//! `finance.receivables.customer.list`; Add chooses or creates the Core
//! company (`company_pick`) and submits `customer.create`, which records
//! the company, Finance customer and relationship in one transaction.
use super::{
    company_pick::{Chooser, Submitted},
    format::{self as f, Errors},
    render, Flash, Pages, CUSTOMERS,
};
use crate::Customer;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BuiltinColumn, Button, ButtonVariant, Cell, Choice, ColumnsOutcome, CreateButton,
    EmptyState, Field, Filter, FilterToolbar, FlexRow, FlexTable, Form, Header, Html, InputKind,
    ListColumns, ListSpec, ListState, ModalWidth, PageCounts, PageVariant, Pagination, RecordLink,
    SortDir, StatusKind, TableEmpty, ToolbarControl,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

const SIZES: [u64; 3] = [25, 50, 100];
static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        sortable: BTreeMap::from([("code".into(), SortDir::Asc), ("name".into(), SortDir::Asc)]),
        default_sort: Some("code".into()),
        page_sizes: SIZES.to_vec(),
        filters: [(
            "status".into(),
            Filter::one_of(&["", "active", "inactive", "unlinked"], ""),
        )]
        .into(),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("customers list spec")
});

/// The customer's own fields in the Add customer form:
/// `(key, label, kind, maximum, required, hint)`.
pub(super) const FIELDS: &[(&str, &str, InputKind, &str, bool, &str)] = &[
    (
        "code",
        "Code",
        InputKind::Text,
        "50",
        true,
        "A short name for this customer in Finance. It cannot be changed later.",
    ),
    (
        "payment_terms_days",
        "Payment terms (days)",
        InputKind::Number,
        "4",
        false,
        "Days after the invoice date that payment is due. Leave empty for due on issue.",
    ),
];
const FORM: &str = "customer";
const PREFIX: &str = "receivables-customer-new";

/// The Add customer dialog while it is open.
struct Create {
    values: HashMap<String, String>,
    chooser: Chooser,
    errors: Errors,
}

pub(super) struct Customers {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    all: Vec<Customer>,
    cut: bool,
    can_add: bool,
    create: Option<Create>,
    flash: Flash,
    framed: bool,
}

impl Customers {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("code", "Code").sortable("code"),
            BuiltinColumn::new("name", "Name").sortable("name"),
            BuiltinColumn::new("company", "Company"),
            BuiltinColumn::new("email", "Email"),
            BuiltinColumn::new("phone", "Phone"),
            BuiltinColumn::new("terms", "Terms").right(),
            BuiltinColumn::new("status", "Status"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("receivables-customers-table", columns)
                .map_err(|_| Error::Internal)?,
            all: vec![],
            cut: false,
            can_add: false,
            create: None,
            flash: Flash::default(),
            framed,
        })
    }

    pub(super) async fn first(&mut self, scope: &Scope) -> Result<String> {
        self.load(scope).await?;
        self.body()
    }

    async fn load(&mut self, scope: &Scope) -> Result<()> {
        (self.all, self.cut) = self
            .pages
            .read_all(
                scope,
                "finance.receivables.customer.list",
                json!({}),
                ("after_code", "next_after_code"),
            )
            .await?;
        self.can_add = self.pages.allowed(scope, crate::MANAGE_CUSTOMERS).await?;
        Ok(())
    }

    /// The rows the filters, search and sort leave, and the page asked for.
    fn shown(&mut self) -> (Vec<&Customer>, u64) {
        let search = self.list.search.trim().to_lowercase();
        let status = self.list.filters.get("status").cloned().unwrap_or_default();
        let mut rows: Vec<&Customer> = self
            .all
            .iter()
            .filter(|c| match status.as_str() {
                "active" => c.active,
                "inactive" => !c.active,
                "unlinked" => c.company.is_none(),
                _ => true,
            })
            .filter(|c| {
                search.is_empty()
                    || [Some(&c.code), Some(&c.name), email(c), phone(c)]
                        .into_iter()
                        .flatten()
                        .any(|t| t.to_lowercase().contains(&search))
            })
            .collect();
        match self.list.sort_by.as_deref() {
            Some("name") => rows.sort_by_key(|c| c.name.to_lowercase()),
            _ => rows.sort_by(|a, b| a.code.cmp(&b.code)),
        }
        if self.list.sort_dir == Some(SortDir::Desc) {
            rows.reverse();
        }
        let total = rows.len() as u64;
        let size = self.list.page_size.max(1);
        self.list.page = self.list.page.clamp(1, total.div_ceil(size).max(1));
        let from = ((self.list.page - 1) * size) as usize;
        (
            rows.into_iter().skip(from).take(size as usize).collect(),
            total,
        )
    }

    fn body(&mut self) -> Result<String> {
        let framed = self.framed;
        let (rows, total) = self.shown();
        let shown: Vec<Customer> = rows.into_iter().cloned().collect();
        self.columns.load(&shown, |c| c.id.to_string());
        let actions = if self.can_add {
            render(CreateButton {
                noun: "Customer",
                id: "receivables-customers-add",
                event: "show_create",
                ..CreateButton::default()
            })?
        } else {
            Html::EMPTY
        };
        let mut content = render(FilterToolbar {
            event: "filter",
            ..FilterToolbar::new(
                "receivables-customers-filters",
                vec![
                    ToolbarControl::search(
                        "receivables-customers-search",
                        "search",
                        "Search customers",
                        &self.list.search,
                        "Search by code, name, email or phone…",
                    ),
                    ToolbarControl::select(
                        "receivables-customers-status",
                        "status",
                        "Filter by status",
                        &self.list.filters["status"],
                        [
                            ("All customers", ""),
                            ("Active", "active"),
                            ("Inactive", "inactive"),
                            ("Not linked to a company", "unlinked"),
                        ]
                        .map(|(l, v)| Choice::new(l, v))
                        .to_vec(),
                    ),
                ],
            )
        })?;
        let mut flex_rows = Vec::new();
        for c in &shown {
            let mut row = FlexRow::new(c.id.to_string());
            row.dom_id = Some(format!("receivables-customer-{}", c.id));
            flex_rows.push(
                row.slot(
                    "code",
                    render(RecordLink {
                        workspace: framed,
                        kind: crate::MODULE,
                        record_id: &c.id.to_string(),
                        href: &format!("{CUSTOMERS}/{}", c.id),
                        class: "font-medium text-ink-strong hover:underline",
                        body: Html::text(&c.code),
                    })?,
                )
                .cell("name", Cell::text(&c.name))
                .slot("company", company_badge(c)?)
                .cell("email", Cell::text(f::optional(&email(c).cloned())))
                .cell("phone", Cell::text(f::optional(&phone(c).cloned())))
                .slot(
                    "terms",
                    f::tabular(&format!("{} days", c.payment_terms_days))?,
                )
                .slot("status", f::active_badge(c.active)?),
            );
        }
        let mut table = FlexTable::new(
            "receivables-customers-table",
            self.columns.flex_columns(),
            flex_rows,
        )
        .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.caption = "Customers";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(render(EmptyState::missing(
            "No customers",
            if self.all.is_empty() {
                "Add the companies you invoice, then raise invoices to them."
            } else {
                "No customer matches the current filters."
            },
        ))?));
        content.push(&render(table)?);
        content.push(&render(Pagination {
            filters_event: "filter",
            ..Pagination::new(
                "receivables-customers-pagination",
                PageCounts::new(self.list.page, self.list.page_size, total),
                &SIZES,
            )
        })?);
        if self.cut {
            content.push(&Html::text(
                "Only the first customers are listed. Search to narrow them.",
            ));
        }
        let header = render(Header {
            title: Html::text("Customers"),
            title_actions: super::menu_pin(
                "receivables-customers-pin",
                "nav-finance-receivables-customers",
                "Customers",
            )?,
            actions,
            ..Header::default()
        })?;
        let mut body = header;
        body.push(&content);
        let modal = match &self.create {
            Some(create) => Some(self.dialog(create)?),
            None => None,
        };
        f::frame(
            "receivables-customers",
            PageVariant::List,
            &self.flash,
            body,
            modal,
        )
    }

    fn dialog(&self, create: &Create) -> Result<Html> {
        let mut labels: Vec<_> = FIELDS
            .iter()
            .map(|(key, label, ..)| {
                (
                    (*key).to_owned(),
                    format!("{PREFIX}-{key}"),
                    (*label).to_owned(),
                )
            })
            .collect();
        labels.extend(create.chooser.labels());
        let mut body = render(create.errors.summary(&format!("{PREFIX}-errors"), &labels))?;
        let mut own = Html::EMPTY;
        for (key, label, kind, max, required, hint) in FIELDS {
            let mut field = Field::in_form(FORM, key)
                .id(format!("{PREFIX}-{key}"))
                .label(*label)
                .kind(*kind)
                .value(create.values.get(*key).cloned().unwrap_or_default())
                .errors(create.errors.under(key, label));
            field = if *kind == InputKind::Number {
                field.attr("min", "0").attr("max", "3650").attr("step", "1")
            } else {
                field.attr("maxlength", max)
            };
            if *required {
                field = field.required();
            }
            if !hint.is_empty() {
                field = field.hint(*hint);
            }
            own.push(&render(field)?);
        }
        body.push(&f::group(
            "receivables-customer-company",
            "Company",
            create.chooser.render(&create.errors, self.framed)?,
        )?);
        body.push(&f::group("receivables-customer-terms", "In Finance", own)?);
        let actions = {
            let mut a = render(Button {
                id: &format!("{PREFIX}-cancel"),
                button_type: "button",
                event: "cancel_create",
                body: Html::text("Cancel"),
                ..Button::default()
            })?;
            a.push(&render(Button {
                id: &format!("{PREFIX}-save"),
                button_type: "button",
                event: "create_customer",
                variant: ButtonVariant::Primary,
                working: "Adding…",
                body: Html::text("Add customer"),
                ..Button::default()
            })?);
            f::actions(a)?
        };
        let form = render(Form {
            id: &format!("{PREFIX}-form"),
            event: "customer_change",
            change: true,
            class: "mt-4",
            body,
            actions,
        })?;
        f::modal(
            "receivables-customer-new-modal",
            "Add customer",
            "",
            "cancel_create",
            ModalWidth::Wide,
            &self.flash,
            form,
        )
    }

    /// The dialog's own values and the company chooser's, from a posted form.
    async fn take_form(&mut self, scope: &Scope, v: &serde_json::Value) -> Result<()> {
        let form = params::form_values(v, FORM);
        if form.is_empty() {
            return Ok(());
        }
        let pages = self.pages.clone();
        let Some(create) = self.create.as_mut() else {
            return Ok(());
        };
        for (key, ..) in FIELDS {
            if let Some(value) = form.get(*key) {
                create.values.insert((*key).to_owned(), value.clone());
            }
        }
        if create.chooser.read(&form) {
            create
                .chooser
                .search(&pages.company_picker(), scope)
                .await?;
        }
        create.chooser.offer_code(&mut create.values);
        Ok(())
    }

    async fn add(&mut self, scope: &Scope, credential: &str) -> Result<Option<String>> {
        let pages = self.pages.clone();
        let Some(create) = self.create.as_mut() else {
            return Ok(None);
        };
        let mut input = json!({});
        let mut local = Errors::default();
        let code = f::posted(&create.values, "code");
        f::put(&mut input, "code", code);
        let terms = f::posted(&create.values, "payment_terms_days");
        if !terms.is_empty() {
            match terms.parse::<i64>() {
                Ok(n) => input["payment_terms_days"] = json!(n),
                Err(_) => local
                    .0
                    .entry("payment_terms_days".into())
                    .or_default()
                    .push("must be a whole number of days".into()),
            }
        }
        if code.is_empty() {
            local
                .0
                .entry("code".into())
                .or_default()
                .push("can't be blank".into());
        }
        if !local.is_empty() {
            create.errors = local;
            return Ok(None);
        }
        match create
            .chooser
            .submit_composed(
                &pages.company_picker(),
                scope,
                credential,
                "create_customer",
                "finance.receivables.customer.create",
                input,
            )
            .await?
        {
            Submitted::Review => {
                create.errors = Errors::default();
                Ok(None)
            }
            Submitted::Done(record) => {
                self.create = None;
                self.flash.set(StatusKind::Success, "Customer added.");
                self.load(scope).await?;
                Ok(record["id"].as_i64().map(|id| format!("{CUSTOMERS}/{id}")))
            }
            Submitted::Invalid(fields) => {
                create.errors = fields;
                Ok(None)
            }
            Submitted::Refused(message) => {
                self.create = None;
                self.flash.set(
                    StatusKind::Error,
                    super::worded(message, "You do not have permission to add customers."),
                );
                Ok(None)
            }
        }
    }

    async fn event(
        &mut self,
        scope: &Scope,
        credential: &str,
        v: &serde_json::Value,
    ) -> Result<LiveBody> {
        let mut redirect = None;
        let pages = self.pages.clone();
        match v["type"].as_str().ok_or(Error::Invalid)? {
            "clear-flash" => self.flash.dismiss(v),
            "filter" => self.list.apply_filters(&params::form_values(v, "filters")),
            "page" => self.list.put_page(v["page"].as_str()),
            "grid" => {
                if let ColumnsOutcome::Sort(column) = self.columns.handle(&params::string_values(v))
                {
                    self.list.next_sort(&column);
                }
            }
            "show_create" => {
                if self.pages.allowed(scope, crate::MANAGE_CUSTOMERS).await? {
                    let (customers, _) = self
                        .pages
                        .read_all::<Customer>(
                            scope,
                            "finance.receivables.customer.list",
                            json!({}),
                            ("after_code", "next_after_code"),
                        )
                        .await?;
                    let existing = customers
                        .into_iter()
                        .filter_map(|customer| {
                            Some((
                                customer.company?.id.to_string(),
                                format!("/finance/receivables/customers/{}", customer.id),
                            ))
                        })
                        .collect();
                    self.create = Some(Create {
                        values: HashMap::new(),
                        chooser: Chooser::new(PREFIX, FORM).existing(
                            "Already a customer",
                            "View customer",
                            existing,
                        ),
                        errors: Errors::default(),
                    });
                } else {
                    self.flash.set(StatusKind::Error, super::DENIED);
                }
            }
            "cancel_create" => self.create = None,
            "customer_change" => self.take_form(scope, v).await?,
            "pick_company" => {
                let record = v["record"].as_str().ok_or(Error::Invalid)?;
                if let Some(create) = self.create.as_mut() {
                    create
                        .chooser
                        .pick(&pages.company_picker(), scope, record)
                        .await?;
                    create.chooser.offer_code(&mut create.values);
                    create.errors.0.remove("company");
                }
            }
            "clear_company" => {
                if let Some(create) = self.create.as_mut() {
                    create.chooser.clear();
                    create.chooser.offer_code(&mut create.values);
                }
            }
            "use_candidate" => {
                let id = v["company"].as_str().ok_or(Error::Invalid)?;
                if let Some(create) = self.create.as_mut() {
                    create
                        .chooser
                        .use_candidate(&pages.company_picker(), scope, id)
                        .await?;
                    create.chooser.offer_code(&mut create.values);
                }
            }
            "create_customer" => {
                self.take_form(scope, v).await?;
                redirect = self.add(scope, credential).await?;
            }
            _ => return Err(Error::Invalid),
        }
        let flash = redirect.is_some().then(|| self.flash.0.take()).flatten();
        Ok(LiveBody {
            html: self.body()?,
            query: self.list.to_params(),
            redirect,
            flash,
            ..LiveBody::default()
        })
    }
}

/// The customer's email: its company's, or its own before linking.
fn email(c: &Customer) -> Option<&String> {
    c.company
        .as_ref()
        .map_or(c.email.as_ref(), |company| company.email.as_ref())
}

fn phone(c: &Customer) -> Option<&String> {
    c.company
        .as_ref()
        .map_or(c.phone.as_ref(), |company| company.phone.as_ref())
}

/// The company's status in Core, or that the customer has no company yet.
pub(super) fn company_badge(c: &Customer) -> Result<Html> {
    use bilibi_ui::BadgeKind;
    match &c.company {
        None => f::badge("Not linked", BadgeKind::Warning),
        Some(company) => match company.status.as_str() {
            "active" => f::badge("Active", BadgeKind::Success),
            "pending" => f::badge("Pending", BadgeKind::Warning),
            "suspended" => f::badge("Suspended", BadgeKind::Warning),
            "archived" => f::badge("Archived", BadgeKind::Neutral),
            _ => f::badge("Unavailable", BadgeKind::Neutral),
        },
    }
}

impl LiveState<Scope, Error> for Customers {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh => self.load(r.scope).await?,
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                }
                LiveInput::Event(v) => return self.event(r.scope, r.credential, v).await,
            }
            Ok(LiveBody {
                html: self.body()?,
                query: self.list.to_params(),
                ..LiveBody::default()
            })
        })
    }
}
