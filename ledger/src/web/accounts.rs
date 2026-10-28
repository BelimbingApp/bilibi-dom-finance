//! The chart of accounts: every account and account group of the company's
//! book with its balance or group total, a dialog to add an account, and the
//! starter charts a company with an empty chart can preview and apply. Each
//! account's page is `account.rs`; groups are managed on `groups.rs`.
use super::{
    account_name, capability_for, dialog_actions, error_text, event_type, figure, no_company,
    render, shell, unavailable, view, Failure, Flash, Pages, Read, Refusal, Review,
};
use crate::{AccountType, MANAGE_ACCOUNTS};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    list_state::FILTERS_FORM,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Card, Cell, Choice,
    ColumnsOutcome, Combobox, CreateButton, EmptyState, Field, Filter, FilterToolbar, FlexRow,
    FlexTable, Form, Header, Html, InputKind, ListColumns, ListSpec, ListState, Modal, PageCounts,
    PageVariant, Pagination, RecordLink, SectionHeading, SortDir, StatusKind, TableEmpty,
    ToolbarControl,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

const SIZES: [u64; 4] = [25, 50, 100, 300];
pub(super) const INDEX: &str = "/finance/ledger/accounts";
const TEMPLATE_CONTRACT: &str = "finance.ledger.chart_template.apply";

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        sortable: BTreeMap::from([
            ("code".into(), SortDir::Asc),
            ("name".into(), SortDir::Asc),
            ("type".into(), SortDir::Asc),
        ]),
        default_sort: Some("code".into()),
        page_sizes: SIZES.to_vec(),
        default_page_size: 50,
        page_size_param: "per_page".into(),
        filters: BTreeMap::from([
            (
                "type".into(),
                Filter::one_of(
                    &["all", "asset", "liability", "equity", "income", "expense"],
                    "all",
                ),
            ),
            (
                "kind".into(),
                Filter::one_of(&["all", "account", "group"], "all"),
            ),
            (
                "status".into(),
                Filter::one_of(&["all", "active", "inactive"], "all"),
            ),
        ]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("chart of accounts list spec")
});

/// The new-account dialog's values and what the ledger said about them.
#[derive(Default)]
struct Draft {
    code: String,
    name: String,
    account_type: String,
    parent: String,
    retained: bool,
    refusal: Refusal,
    notice: String,
}

impl Draft {
    fn new() -> Self {
        Self {
            account_type: "asset".into(),
            ..Self::default()
        }
    }
    fn read(&mut self, value: &Value) {
        let fields = params::form_values(value, "account");
        self.code = fields.get("code").cloned().unwrap_or_default();
        self.name = fields.get("name").cloned().unwrap_or_default();
        self.account_type = fields.get("account_type").cloned().unwrap_or_default();
        self.parent = fields.get("parent").cloned().unwrap_or_default();
        self.retained = fields.get("retained").map(String::as_str) == Some("true");
    }
    fn equity(&self) -> bool {
        self.account_type == "equity"
    }
    /// Whether `account` can hold an account of the chosen type: an account
    /// group of the same type, as the ledger requires.
    fn can_group(&self, account: &view::Account) -> bool {
        account.is_group && account.account_type == self.account_type
    }
    /// Drops what the chosen type no longer allows, after the person changed
    /// it: a group of another type, and the retained-earnings designation on
    /// anything but an equity account. The ledger's own check still decides.
    fn reconcile(&mut self, chart: &[view::Account]) {
        if !self.equity() {
            self.retained = false;
        }
        let eligible = self
            .parent
            .parse::<i64>()
            .ok()
            .and_then(|id| chart.iter().find(|a| a.id == id))
            .is_some_and(|parent| self.can_group(parent));
        if !eligible {
            self.parent.clear();
        }
    }
    fn input(&self) -> Value {
        let mut input = json!({
            "code": self.code.trim(),
            "name": self.name.trim(),
            "account_type": self.account_type,
        });
        if let Ok(parent) = self.parent.parse::<i64>() {
            input["parent_id"] = json!(parent);
        }
        // A designation ticked while the type was equity never travels with
        // another type.
        if self.retained && self.equity() {
            input["retained_earnings"] = json!(true);
        }
        input
    }
}

/// A starter chart the person is looking at before applying it: the
/// contract's own review of exactly what would be created.
struct Preview {
    review: Review,
    name: String,
    status: String,
    notes: Vec<String>,
    rows: Vec<view::TemplateRow>,
}

pub(super) struct Accounts {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    template_columns: ListColumns,
    flash: Flash,
    draft: Option<Draft>,
    preview: Option<Preview>,
    framed: bool,
}

impl Accounts {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("code", "Code").sortable("code"),
            BuiltinColumn::new("name", "Name").sortable("name"),
            BuiltinColumn::new("kind", "Kind"),
            BuiltinColumn::new("type", "Type").sortable("type"),
            BuiltinColumn::new("group", "Group"),
            BuiltinColumn::new("balance", "Balance"),
            BuiltinColumn::new("total", "Group total"),
            BuiltinColumn::new("status", "Status"),
            BuiltinColumn::new("retained", "Retained earnings"),
        ];
        let template_columns = vec![
            BuiltinColumn::new("code", "Code"),
            BuiltinColumn::new("name", "Name"),
            BuiltinColumn::new("kind", "Kind"),
            BuiltinColumn::new("type", "Type"),
            BuiltinColumn::new("group", "Group"),
            BuiltinColumn::new("retained", "Retained earnings"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("ledger-accounts-table", columns)
                .map_err(|_| Error::Internal)?,
            template_columns: ListColumns::mount("ledger-template-table", template_columns)
                .map_err(|_| Error::Internal)?,
            flash: Flash::default(),
            draft: query.get("new").is_some().then(Draft::new),
            preview: None,
            framed,
        })
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let can_manage = self.pages.allowed(scope, MANAGE_ACCOUNTS).await?;
        let mut body = Html::EMPTY;
        let mut actions = Html::EMPTY;
        let mut rows_shown = 0;
        let mut chart: Vec<view::Account> = Vec::new();
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-accounts-no-company")?);
        } else {
            match self.pages.chart(scope).await? {
                Err(refused) => body.push(&unavailable(
                    "ledger-accounts-refused",
                    "the chart of accounts",
                    &refused,
                )?),
                Ok(listed) => {
                    chart = listed;
                    let book = self.pages.book(scope).await?.ok();
                    let book_open = book.as_ref().is_some_and(|b| b.book.is_some());
                    if can_manage && book_open {
                        actions.push(&render(Button {
                            id: "ledger-account-groups-link",
                            href: super::groups::INDEX,
                            class: "text-xs",
                            body: Html::text("Account groups"),
                            ..Button::default()
                        })?);
                        actions.push(&render(CreateButton {
                            noun: "Account",
                            id: "ledger-account-new",
                            class: "text-xs",
                            event: "new",
                            button_type: "button",
                            ..CreateButton::default()
                        })?);
                    }
                    if !book_open && chart.is_empty() {
                        let mut empty = EmptyState::missing(
                            "The book is not open yet",
                            "Open the company's book first. It sets the currency every account and entry uses.",
                        )
                        .id("ledger-accounts-no-book");
                        empty = empty.action(render(Button {
                            id: "ledger-accounts-open-book",
                            href: super::book::INDEX,
                            body: Html::text("Go to the book"),
                            variant: ButtonVariant::Primary,
                            ..Button::default()
                        })?);
                        body.push(&render(empty)?);
                    } else if chart.is_empty() && can_manage {
                        body.push(&self.starters(scope).await?);
                    } else {
                        let figures = if book_open {
                            self.pages.chart_figures(scope).await?
                        } else {
                            None
                        };
                        rows_shown = self.table(&chart, figures.as_ref(), &mut body)?;
                    }
                }
            }
        }
        let mut page = render(Header {
            title: Html::text("Chart of accounts"),
            title_actions: super::menu_pin(
                "ledger-accounts-pin",
                "nav-finance-ledger-accounts",
                "Chart of accounts",
            )?,
            actions,
            ..Header::default()
        })?;
        page.push(&body);
        if let Some(draft) = &mut self.draft {
            draft.reconcile(&chart);
        }
        if self.draft.is_some() && can_manage {
            let form = self.form(&chart)?;
            page.push(&render(Modal {
                id: "ledger-account-modal",
                title: "New account",
                cancel_event: "close_modal",
                body: form,
                ..Modal::default()
            })?);
        }
        let mut query = self.list.to_params();
        if self.draft.is_some() {
            query.insert("new".into(), "1".into());
        }
        Ok(LiveBody {
            html: shell("ledger-accounts", PageVariant::List, &self.flash, page)?,
            query,
            rows: rows_shown,
            ..LiveBody::default()
        })
    }

    fn table(
        &mut self,
        chart: &[view::Account],
        figures: Option<&view::ChartBalances>,
        body: &mut Html,
    ) -> Result<usize> {
        let search = self.list.search.trim().to_lowercase();
        let by_id: HashMap<i64, &view::Account> = chart.iter().map(|a| (a.id, a)).collect();
        let mut shown: Vec<&view::Account> = chart
            .iter()
            .filter(|a| {
                let kind = self.list.filters["type"].as_str();
                let status = self.list.filters["status"].as_str();
                (kind == "all" || a.account_type == kind)
                    && match self.list.filters["kind"].as_str() {
                        "account" => !a.is_group,
                        "group" => a.is_group,
                        _ => true,
                    }
                    && match status {
                        "active" => a.active,
                        "inactive" => !a.active,
                        _ => true,
                    }
                    && (search.is_empty()
                        || a.code.to_lowercase().contains(&search)
                        || a.name.to_lowercase().contains(&search))
            })
            .collect();
        let descending = self.list.sort_dir == Some(SortDir::Desc);
        match self.list.sort_by.as_deref() {
            Some("name") => shown.sort_by_key(|a| a.name.to_lowercase()),
            Some("type") => shown.sort_by(|a, b| {
                (a.account_type.as_str(), a.code.as_str())
                    .cmp(&(b.account_type.as_str(), b.code.as_str()))
            }),
            _ => shown.sort_by(|a, b| a.code.cmp(&b.code)),
        }
        if descending {
            shown.reverse();
        }
        let total = shown.len() as u64;
        self.list
            .clamp_to_last_page(total.div_ceil(self.list.page_size), true);
        let start = ((self.list.page - 1) * self.list.page_size) as usize;
        let page: Vec<&view::Account> = shown
            .into_iter()
            .skip(start)
            .take(self.list.page_size as usize)
            .collect();
        self.columns.load(&page, |a| a.id.to_string());

        let form = self.list.filters_form();
        body.push(&render(FilterToolbar::new(
            "ledger-accounts-filters",
            vec![
                ToolbarControl::search(
                    "ledger-accounts-search",
                    "search",
                    "Search accounts",
                    &form["search"],
                    "Search by code or name…",
                ),
                ToolbarControl::select(
                    "ledger-accounts-type",
                    "type",
                    "Type",
                    &form["type"],
                    vec![
                        Choice::new("All types", "all"),
                        Choice::new("Asset", "asset"),
                        Choice::new("Liability", "liability"),
                        Choice::new("Equity", "equity"),
                        Choice::new("Income", "income"),
                        Choice::new("Expense", "expense"),
                    ],
                ),
                ToolbarControl::select(
                    "ledger-accounts-kind",
                    "kind",
                    "Kind",
                    &form["kind"],
                    vec![
                        Choice::new("Accounts and groups", "all"),
                        Choice::new("Accounts", "account"),
                        Choice::new("Groups", "group"),
                    ],
                ),
                ToolbarControl::select(
                    "ledger-accounts-status",
                    "status",
                    "Status",
                    &form["status"],
                    vec![
                        Choice::new("All accounts", "all"),
                        Choice::new("Active", "active"),
                        Choice::new("Inactive", "inactive"),
                    ],
                ),
            ],
        ))?);

        let units = figures.map(|f| f.minor_units);
        let figure_of = |id: i64| figures.and_then(|f| f.rows.iter().find(|r| r.id == id));
        let mut rows = Vec::new();
        for account in &page {
            let group = match account.parent_id.and_then(|id| by_id.get(&id)) {
                Some(parent) => account_name(parent)?,
                None => Html::text("—"),
            };
            let href = if account.is_group {
                format!("{}/{}", super::groups::INDEX, account.id)
            } else {
                format!("{INDEX}/{}", account.id)
            };
            let shown = |text: Option<&String>| -> Result<Html> {
                match (text, units) {
                    (Some(text), Some(units)) => figure(&view::amount(text, units)),
                    _ => Ok(Html::text("—")),
                }
            };
            let own = figure_of(account.id);
            rows.push({
                let mut row = FlexRow::new(account.id.to_string())
                    .slot(
                        "code",
                        render(RecordLink {
                            workspace: self.framed,
                            kind: crate::MODULE,
                            record_id: &account.id.to_string(),
                            href: &href,
                            class: "font-medium text-ink-strong hover:underline",
                            body: Html::text(&account.code),
                        })?,
                    )
                    .cell("name", Cell::text(&account.name))
                    .cell(
                        "kind",
                        Cell::text(if account.is_group { "Group" } else { "Account" }),
                    )
                    .cell("type", Cell::text(view::type_label(&account.account_type)))
                    .slot("group", group)
                    .slot("balance", shown(own.and_then(|r| r.balance.as_ref()))?)
                    .slot("total", shown(own.and_then(|r| r.total.as_ref()))?)
                    .slot(
                        "status",
                        if account.is_group {
                            Html::text("—")
                        } else {
                            status_badge(account.active)?
                        },
                    )
                    .cell(
                        "retained",
                        Cell::text(if account.retained_earnings {
                            "Yes"
                        } else {
                            "—"
                        }),
                    );
                row.dom_id = Some(format!("ledger-account-{}", account.id));
                row
            });
        }
        let mut table = FlexTable::new("ledger-accounts-table", self.columns.flex_columns(), rows)
            .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.caption = "Chart of accounts";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(Html::text(
            "Nothing matches. Change the filters, or add an account.",
        )));
        let mut card = render(table)?;
        card.push(&render(Pagination::new(
            "ledger-accounts-pagination",
            PageCounts::new(self.list.page, self.list.page_size, total),
            &SIZES,
        ))?);
        body.push(&render(Card {
            id: "ledger-accounts-card",
            inner_class: "p-0",
            body: card,
            ..Card::default()
        })?);
        Ok(page.len())
    }

    fn form(&self, chart: &[view::Account]) -> Result<Html> {
        let draft = self.draft.as_ref().ok_or(Error::Internal)?;
        let type_word = view::type_label(&draft.account_type).to_lowercase();
        // Only an account group of the same type can hold the new account.
        let eligible = view::by_name(chart.iter().filter(|a| draft.can_group(a)));
        let mut parents = vec![Choice::new("No group", "")];
        parents.extend(eligible.iter().map(|a| view::account_choice(a)));
        let parent_hint = if eligible.is_empty() {
            format!("Optional. This company has no {type_word} account group yet. Add one on Account groups.")
        } else {
            format!("Optional. An account group of the same type ({type_word}).")
        };
        // Designating a new account releases the account that holds retained
        // earnings now, so the dialog says which one before the person ticks it.
        let retained_hint = if !draft.equity() {
            "Only an equity account can hold retained earnings.".to_owned()
        } else {
            match chart.iter().find(|a| a.retained_earnings) {
                Some(holder) => format!(
                    "The year-end close posts profit or loss here. This replaces {}, which holds retained earnings now.",
                    view::account_label(holder)
                ),
                None => "The year-end close posts profit or loss here.".to_owned(),
            }
        };
        let errors = |name: &str| {
            draft
                .refusal
                .field(name)
                .iter()
                .map(|m| error_text(name, m))
                .collect::<Vec<_>>()
        };
        let field = |key: &str, label: &str, value: &str| {
            Field::in_form("account", key)
                .id(format!("ledger-account-{key}"))
                .label(label)
                .value(value)
                .errors(errors(key))
                .attr("data-debounce", "300")
        };
        let mut body = Html::EMPTY;
        if !draft.notice.is_empty() {
            body.push(&render(Alert {
                id: "ledger-account-notice",
                kind: StatusKind::Error,
                body: Html::text(&draft.notice),
                ..Alert::default()
            })?);
        }
        let base = draft.refusal.alert(&[
            "code",
            "name",
            "account_type",
            "parent_id",
            "retained_earnings",
        ]);
        if !base.is_empty() {
            body.push(&render(Alert {
                id: "ledger-account-base",
                kind: StatusKind::Error,
                body: Html::text(&base.join(" ")),
                ..Alert::default()
            })?);
        }
        for f in [
            field("code", "Code", &draft.code)
                .required()
                .hint("Up to 50 characters. The code cannot be changed later."),
            field("name", "Name", &draft.name).required(),
            field("account_type", "Type", &draft.account_type)
                .kind(InputKind::Select)
                .options(
                    AccountType::ALL
                        .iter()
                        .map(|t| Choice::new(view::type_label(t.as_str()), t.as_str()))
                        .collect(),
                )
                .hint("The type cannot be changed later."),
        ] {
            body.push(&render(f)?);
        }
        let parent = field("parent", "Group under", &draft.parent);
        body.push(&render(
            Combobox::new(parent.id.clone(), parent.name.clone())
                .label("Group under")
                .placeholder("No group")
                .value(&draft.parent)
                .options(parents)
                .errors(errors("parent_id"))
                .hint(parent_hint),
        )?);
        let mut retained = field("retained", "Use for retained earnings", "true")
            .errors(errors("retained_earnings"))
            .kind(InputKind::Checkbox)
            .checked(draft.retained && draft.equity())
            .hint(retained_hint);
        if !draft.equity() {
            // Greyed, with the line above saying what unlocks it; a disabled
            // box posts nothing, so no designation rides along.
            retained = retained.disabled();
        }
        body.push(&render(retained)?);
        let actions = {
            let mut a = render(Button {
                id: "ledger-account-cancel",
                button_type: "button",
                event: "close_modal",
                body: Html::text("Cancel"),
                ..Button::default()
            })?;
            a.push(&render(Button {
                id: "ledger-account-create",
                button_type: "button",
                event: "create",
                variant: ButtonVariant::Primary,
                working: "Adding…",
                body: Html::text("Add account"),
                ..Button::default()
            })?);
            a
        };
        render(Form {
            id: "ledger-account-form",
            event: "validate",
            change: true,
            class: "mt-4",
            body,
            actions: dialog_actions(actions)?,
        })
    }
}

pub(super) fn status_badge(active: bool) -> Result<Html> {
    render(Badge::status(
        if active {
            BadgeKind::Success
        } else {
            BadgeKind::Neutral
        },
        if active { "Active" } else { "Inactive" },
    ))
}

impl LiveState<Scope, Error> for Accounts {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh => {}
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                    if values.get("new").is_some() && self.draft.is_none() {
                        self.draft = Some(Draft::new());
                    }
                }
                LiveInput::Event(value) => match event_type(value)? {
                    "filters" => self
                        .list
                        .apply_filters(&params::form_values(value, FILTERS_FORM)),
                    "page" => self.list.put_page(value["page"].as_str()),
                    "sort" => {
                        self.list.next_sort(value["sort"].as_str().unwrap_or(""));
                    }
                    "grid" => {
                        if let ColumnsOutcome::Sort(key) =
                            self.columns.handle(&params::string_values(value))
                        {
                            self.list.next_sort(&key);
                        }
                    }
                    "new" => {
                        if self
                            .pages
                            .allowed(r.scope, capability_for("create_account"))
                            .await?
                        {
                            self.draft = Some(Draft::new());
                        }
                    }
                    "close_modal" => self.draft = None,
                    "ask_template" => self.ask_template(&r, value).await?,
                    "apply_template" => self.apply_template(&r).await?,
                    "cancel_template" => self.preview = None,
                    "validate" => {
                        if let Some(draft) = &mut self.draft {
                            draft.read(value);
                            draft.refusal = Refusal::default();
                            draft.notice.clear();
                        }
                    }
                    "create" => self.create(&r).await?,
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                },
            }
            self.view(r.scope).await
        })
    }
}

impl Accounts {
    /// What an empty chart offers: each starter chart to preview, or an empty
    /// start (the header's New account, and Account groups). The chart is
    /// never filled in before a person chooses.
    async fn starters(&mut self, scope: &Scope) -> Result<Html> {
        if self.preview.is_some() {
            return self.preview_card();
        }
        let listed = match self
            .pages
            .read(scope, "finance.ledger.chart_template.list", json!({}))
            .await?
        {
            Read::Found(value) => {
                view::parse::<Vec<view::TemplateSummary>>(value["records"].clone())
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let mut body = render(SectionHeading {
            id: "ledger-starters-heading",
            title: "Start your chart of accounts",
            ..SectionHeading::default()
        })?;
        body.push(&render(Note {
            id: "ledger-starters-intro",
            text: "The chart is empty. Preview a starter chart and apply it, or start empty and add account groups and accounts yourself. Nothing is added until you apply.",
        })?);
        let mut html = render(Card {
            id: "ledger-starters-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-starters-heading",
            body,
            ..Card::default()
        })?;
        for template in listed.iter().filter(|t| t.applicable) {
            let mut card = render(SectionHeading {
                id: "ledger-starter-heading",
                title: &template.name,
                ..SectionHeading::default()
            })?;
            card.push(&render(Badge::status(
                BadgeKind::Warning,
                &template.review,
            ))?);
            card.push(&render(Note {
                id: "ledger-starter-description",
                text: &template.description,
            })?);
            card.push(&render(Note {
                id: "ledger-starter-counts",
                text: &format!(
                    "Creates {} account groups and {} accounts. You can rename them afterwards.",
                    template.groups, template.accounts
                ),
            })?);
            card.push(&render(Button {
                id: "ledger-starter-preview",
                button_type: "button",
                event: "ask_template",
                name: "template",
                value: &template.key,
                class: "mt-4",
                variant: ButtonVariant::Primary,
                body: Html::text("Preview this chart"),
                ..Button::default()
            })?);
            html.push(&render(Card {
                id: "ledger-starter-card",
                inner_class: "p-5 sm:p-6",
                role: "region",
                labelledby: "ledger-starter-heading",
                body: card,
                ..Card::default()
            })?);
        }
        Ok(html)
    }

    /// The exact groups and accounts the contract's review says it would
    /// create, with the choice to apply them or leave the chart empty.
    fn preview_card(&mut self) -> Result<Html> {
        let Some(preview) = &self.preview else {
            return Ok(Html::EMPTY);
        };
        let by_code: HashMap<&str, &view::TemplateRow> =
            preview.rows.iter().map(|r| (r.code.as_str(), r)).collect();
        self.template_columns
            .load(&preview.rows, |r| r.code.clone());
        let rows: Vec<FlexRow> = preview
            .rows
            .iter()
            .map(|row| {
                let group = row
                    .parent_code
                    .as_deref()
                    .and_then(|c| by_code.get(c))
                    .map_or_else(|| "—".to_owned(), |p| format!("{} · {}", p.name, p.code));
                FlexRow::new(row.code.clone())
                    .cell("code", Cell::text(&row.code))
                    .cell("name", Cell::text(&row.name))
                    .cell(
                        "kind",
                        Cell::text(if row.is_group { "Group" } else { "Account" }),
                    )
                    .cell("type", Cell::text(view::type_label(&row.account_type)))
                    .cell("group", Cell::text(&group))
                    .cell(
                        "retained",
                        Cell::text(if row.retained_earnings { "Yes" } else { "—" }),
                    )
            })
            .collect();
        let mut table = FlexTable::new(
            "ledger-template-table",
            self.template_columns.flex_columns(),
            rows,
        );
        table.caption = "Groups and accounts this chart would create";
        table.framed = false;
        table.zoom = self.template_columns.zoom;
        table.suggestions = self.template_columns.flex_suggestions();
        table.add_query = &self.template_columns.add_query;
        let mut body = render(SectionHeading {
            id: "ledger-preview-heading",
            title: &preview.name,
            ..SectionHeading::default()
        })?;
        body.push(&render(Badge::status(BadgeKind::Warning, &preview.status))?);
        body.push(&render(Note {
            id: "ledger-preview-note",
            text: "Review every group and account below. Applying creates them as this company's own chart, with the retained earnings account marked. The chart is never changed once it has accounts.",
        })?);
        for (index, note) in preview.notes.iter().enumerate() {
            body.push(&render(Note {
                id: &format!("ledger-preview-note-{index}"),
                text: note,
            })?);
        }
        let mut actions = render(Button {
            id: "ledger-preview-cancel",
            button_type: "button",
            event: "cancel_template",
            body: Html::text("Start empty instead"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "ledger-preview-apply",
            button_type: "button",
            event: "apply_template",
            variant: ButtonVariant::Primary,
            working: "Applying…",
            body: Html::text("Apply this chart"),
            ..Button::default()
        })?);
        body.push(&render(Actions { buttons: actions })?);
        let mut html = render(Card {
            id: "ledger-preview-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-preview-heading",
            body,
            ..Card::default()
        })?;
        html.push(&render(Card {
            id: "ledger-preview-rows",
            inner_class: "p-0",
            body: render(table)?,
            ..Card::default()
        })?);
        Ok(html)
    }

    async fn ask_template(&mut self, r: &LiveRequest<'_, Scope>, value: &Value) -> Result<()> {
        let key = value["template"].as_str().ok_or(Error::Invalid)?;
        match self
            .pages
            .review(
                r.scope,
                r.credential,
                "ask_template",
                TEMPLATE_CONTRACT,
                json!({"template": key}),
            )
            .await?
        {
            Ok(review) => {
                let rows = view::parse::<Vec<view::TemplateRow>>(review.facts["rows"].clone())
                    .unwrap_or_default();
                self.preview = Some(Preview {
                    name: review.facts["template"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    status: review.facts["template"]["review"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    notes: view::parse(review.facts["template"]["notes"].clone())
                        .unwrap_or_default(),
                    rows,
                    review,
                });
            }
            Err(failure) => self.flash.error(failure.text("The starter chart")),
        }
        Ok(())
    }

    async fn apply_template(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(preview) = self.preview.take() else {
            return Ok(());
        };
        match self
            .pages
            .commit(
                r.scope,
                r.credential,
                "apply_template",
                TEMPLATE_CONTRACT,
                preview.review,
            )
            .await?
        {
            Ok(done) => {
                let record = &done["results"][0]["result"]["result"]["record"];
                self.flash.success(format!(
                    "Chart applied: {} account groups and {} accounts.",
                    record["groups"].as_u64().unwrap_or_default(),
                    record["accounts"].as_u64().unwrap_or_default()
                ));
            }
            Err(failure) => self.flash.error(failure.text("The starter chart")),
        }
        Ok(())
    }

    async fn create(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(draft) = &mut self.draft else {
            return Ok(());
        };
        match self
            .pages
            .submit(
                r.scope,
                r.credential,
                "create",
                "finance.ledger.account.create",
                draft.input(),
            )
            .await?
        {
            Ok(done) => {
                let code = done["result"]["record"]["code"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.draft = None;
                self.flash.success(format!("Account {code} added."));
            }
            Err(Failure::Refused(refusal)) => {
                draft.refusal = refusal;
                draft.notice.clear();
            }
            Err(failure) => {
                draft.notice = failure.text("The account");
            }
        }
        Ok(())
    }
}

/// A short explanatory line under a heading.
#[derive(askama::Template)]
#[template(
    source = r#"<p id="{{ id }}" class="mt-2 text-sm text-ink-muted">{{ text }}</p>"#,
    ext = "html"
)]
struct Note<'a> {
    id: &'a str,
    text: &'a str,
}

/// Actions at the end of the row, as dialogs end theirs.
#[derive(askama::Template)]
#[template(
    source = r#"<div class="mt-4 flex w-full flex-wrap justify-end gap-2">{{ buttons|safe }}</div>"#,
    ext = "html"
)]
struct Actions {
    buttons: Html,
}
