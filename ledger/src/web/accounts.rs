//! The chart of accounts: every account of the company's book, and a dialog to
//! add one. Each account's page is `account.rs`.
use super::{
    capability_for, dialog_actions, error_text, event_type, no_company, render, shell, unavailable,
    view, Failure, Flash, Pages, Refusal,
};
use crate::{AccountType, MANAGE_ACCOUNTS};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    list_state::FILTERS_FORM,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Card, Cell, Choice,
    ColumnsOutcome, CreateButton, EmptyState, Field, Filter, FilterToolbar, FlexRow, FlexTable,
    Form, Header, Html, InputKind, ListColumns, ListSpec, ListState, Modal, PageCounts,
    PageVariant, Pagination, RecordLink, SortDir, StatusKind, TableEmpty, ToolbarControl,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

const SIZES: [u64; 4] = [25, 50, 100, 300];
pub(super) const INDEX: &str = "/finance/ledger/accounts";

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
    fn input(&self) -> Value {
        let mut input = json!({
            "code": self.code.trim(),
            "name": self.name.trim(),
            "account_type": self.account_type,
        });
        if let Ok(parent) = self.parent.parse::<i64>() {
            input["parent_id"] = json!(parent);
        }
        if self.retained {
            input["retained_earnings"] = json!(true);
        }
        input
    }
}

pub(super) struct Accounts {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    flash: Flash,
    draft: Option<Draft>,
    framed: bool,
}

impl Accounts {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("code", "Code").sortable("code"),
            BuiltinColumn::new("name", "Name").sortable("name"),
            BuiltinColumn::new("type", "Type").sortable("type"),
            BuiltinColumn::new("group", "Group"),
            BuiltinColumn::new("status", "Status"),
            BuiltinColumn::new("retained", "Retained earnings"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("ledger-accounts-table", columns)
                .map_err(|_| Error::Internal)?,
            flash: Flash::default(),
            draft: query.get("new").is_some().then(Draft::new),
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
                    } else {
                        rows_shown = self.table(&chart, &mut body)?;
                    }
                }
            }
        }
        let mut page = render(Header {
            title: Html::text("Chart of accounts"),
            actions,
            ..Header::default()
        })?;
        page.push(&body);
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

    fn table(&mut self, chart: &[view::Account], body: &mut Html) -> Result<usize> {
        let search = self.list.search.trim().to_lowercase();
        let by_id: HashMap<i64, &view::Account> = chart.iter().map(|a| (a.id, a)).collect();
        let mut shown: Vec<&view::Account> = chart
            .iter()
            .filter(|a| {
                let kind = self.list.filters["type"].as_str();
                let status = self.list.filters["status"].as_str();
                (kind == "all" || a.account_type == kind)
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

        let mut rows = Vec::new();
        for account in &page {
            let group = account
                .parent_id
                .and_then(|id| by_id.get(&id))
                .map_or("—".to_owned(), |p| format!("{} {}", p.code, p.name));
            rows.push({
                let mut row = FlexRow::new(account.id.to_string())
                    .slot(
                        "code",
                        render(RecordLink {
                            workspace: self.framed,
                            kind: crate::MODULE,
                            record_id: &account.id.to_string(),
                            href: &format!("{INDEX}/{}", account.id),
                            class: "font-medium text-ink-strong hover:underline",
                            body: Html::text(&account.code),
                        })?,
                    )
                    .cell("name", Cell::text(&account.name))
                    .cell("type", Cell::text(view::type_label(&account.account_type)))
                    .cell("group", Cell::text(group))
                    .slot("status", status_badge(account.active)?)
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
            "No accounts match. Change the filters, or add an account.",
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
        let mut parents = vec![Choice::new("No group", "")];
        parents.extend(chart.iter().filter(|a| a.active).map(|a| {
            Choice::new(
                format!(
                    "{} {} ({})",
                    a.code,
                    a.name,
                    view::type_label(&a.account_type)
                ),
                a.id.to_string(),
            )
        }));
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
            field("parent", "Group under", &draft.parent)
                .errors(errors("parent_id"))
                .kind(InputKind::Select)
                .options(parents)
                .hint("Optional. A group must be the same type as the account."),
            field("retained", "Use for retained earnings", "true")
                .errors(errors("retained_earnings"))
                .kind(InputKind::Checkbox)
                .checked(draft.retained)
                .hint("For an equity account only. The year-end close posts profit or loss here."),
        ] {
            body.push(&render(f)?);
        }
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
