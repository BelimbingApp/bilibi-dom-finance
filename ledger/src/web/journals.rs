//! Manual journals: adjustments someone submits and someone else approves.
//! An approved journal posts as an entry; a submitted one posts nothing yet.
use super::{
    error_text, event_type, figure, instant, no_company, render, shell, unavailable, view, who,
    Failure, Flash, Pages, Read, Refusal,
};
use crate::SUBMIT_JOURNALS;
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    list_state::FILTERS_FORM,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Card, Cell, Choice,
    ColumnsOutcome, CreateButton, EmptyState, Field, Filter, FilterToolbar, FlexRow, FlexTable,
    Form, Header, Html, InputKind, ListColumns, ListSpec, ListState, Modal, ModalWidth,
    PageVariant, RecordLink, StatusKind, TableEmpty, ToolbarControl,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

pub(super) const INDEX: &str = "/finance/ledger/journals";
const LIMIT: usize = 100;
/// Fields a posted form keeps are bounded (64), so a journal has at most this many lines.
const MAX_LINES: usize = 12;

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        page_sizes: vec![100],
        default_page_size: 100,
        filters: BTreeMap::from([
            (
                "status".into(),
                Filter::one_of(&["all", "submitted", "approved", "rejected"], "all"),
            ),
            ("after".into(), Filter::string("")),
        ]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("manual journals list spec")
});

#[derive(Clone, Default)]
struct LineDraft {
    account: String,
    debit: String,
    credit: String,
    description: String,
}

/// The new-journal dialog's values and what the ledger said about them.
struct Draft {
    date: String,
    description: String,
    reference: String,
    lines: Vec<LineDraft>,
    refusal: Refusal,
    /// For each line sent, which dialog line it was (blank lines are not sent).
    sent: Vec<usize>,
    notice: String,
}

impl Draft {
    fn new(today: String) -> Self {
        Self {
            date: today,
            description: String::new(),
            reference: String::new(),
            lines: vec![LineDraft::default(), LineDraft::default()],
            refusal: Refusal::default(),
            sent: Vec::new(),
            notice: String::new(),
        }
    }

    fn read(&mut self, value: &Value) {
        let fields = params::form_values(value, "journal");
        let get = |key: &str| fields.get(key).cloned().unwrap_or_default();
        self.date = get("entry_date");
        self.description = get("description");
        self.reference = get("reference");
        for (i, line) in self.lines.iter_mut().enumerate() {
            line.account = get(&format!("line{i}_account"));
            line.debit = get(&format!("line{i}_debit"));
            line.credit = get(&format!("line{i}_credit"));
            line.description = get(&format!("line{i}_description"));
        }
        self.refusal = Refusal::default();
        self.notice.clear();
    }

    fn input(&mut self) -> Value {
        self.sent.clear();
        let mut lines = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.account.is_empty()
                && line.debit.trim().is_empty()
                && line.credit.trim().is_empty()
                && line.description.trim().is_empty()
            {
                continue;
            }
            let mut out = json!({});
            if let Ok(account) = line.account.parse::<i64>() {
                out["account_id"] = json!(account);
            } else {
                // The contract needs an account id; zero is refused with its own words.
                out["account_id"] = json!(0);
            }
            if !line.debit.trim().is_empty() {
                out["debit"] = json!(line.debit.trim());
            }
            if !line.credit.trim().is_empty() {
                out["credit"] = json!(line.credit.trim());
            }
            if !line.description.trim().is_empty() {
                out["description"] = json!(line.description.trim());
            }
            lines.push(out);
            self.sent.push(i);
        }
        let mut input = json!({
            "entry_date": self.date,
            "description": self.description.trim(),
            "lines": lines,
        });
        if !self.reference.trim().is_empty() {
            input["reference"] = json!(self.reference.trim());
        }
        input
    }

    /// The messages that belong to one dialog line and one of its fields.
    fn line_errors(&self, line: usize, field: &str) -> Vec<String> {
        let Some(sent) = self.sent.iter().position(|&i| i == line) else {
            return Vec::new();
        };
        self.refusal
            .fields
            .get(&format!("lines.{sent}.{field}"))
            .into_iter()
            .flatten()
            .map(|m| error_text(field, m))
            .collect()
    }

    fn totals(&self) -> (Decimal, Decimal) {
        let sum = |pick: fn(&LineDraft) -> &str| {
            self.lines
                .iter()
                .map(|l| view::decimal(pick(l).trim()))
                .sum::<Decimal>()
        };
        (sum(|l| &l.debit), sum(|l| &l.credit))
    }
}

pub(super) struct Journals {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    flash: Flash,
    draft: Option<Draft>,
    framed: bool,
}

impl Journals {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("number", "Number"),
            BuiltinColumn::new("date", "Date"),
            BuiltinColumn::new("description", "Description"),
            BuiltinColumn::new("reference", "Reference"),
            BuiltinColumn::new("amount", "Amount").right(),
            BuiltinColumn::new("status", "Status"),
            BuiltinColumn::new("by", "Submitted by"),
            BuiltinColumn::new("at", "Submitted"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("ledger-journals-table", columns)
                .map_err(|_| Error::Internal)?,
            flash: Flash::default(),
            draft: None,
            framed,
        })
    }

    fn filter(&self, key: &str) -> &str {
        self.list.filters.get(key).map_or("", String::as_str)
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let can_submit = self.pages.allowed(scope, SUBMIT_JOURNALS).await?;
        let mut body = Html::EMPTY;
        let mut actions = Html::EMPTY;
        let mut shown = 0;
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-journals-no-company")?);
        } else {
            match self.pages.book(scope).await? {
                Err(refused) => body.push(&unavailable(
                    "ledger-journals-refused",
                    "manual journals",
                    &refused,
                )?),
                Ok(state) => match state.book {
                    None => body.push(&render(
                        EmptyState::missing(
                            "The book is not open yet",
                            "Open the company's book before you submit a journal.",
                        )
                        .id("ledger-journals-no-book"),
                    )?),
                    Some(book) => {
                        if can_submit {
                            actions.push(&render(CreateButton {
                                noun: "Journal",
                                id: "ledger-journal-new",
                                class: "text-xs",
                                event: "new",
                                button_type: "button",
                                ..CreateButton::default()
                            })?);
                        }
                        shown = self.table(scope, book.minor_units, &mut body).await?;
                    }
                },
            }
        }
        let mut page = render(Header {
            title: Html::text("Manual journals"),
            subtitle: "Submitted journals post when approved.",
            actions,
            ..Header::default()
        })?;
        page.push(&body);
        if self.draft.is_some() && can_submit {
            let accounts = self.pages.chart(scope).await?.unwrap_or_default();
            let form = self.form(&accounts)?;
            page.push(&render(Modal {
                id: "ledger-journal-modal",
                title: "New manual journal",
                cancel_event: "close_modal",
                width: ModalWidth::Wide,
                description: "Debits and credits must balance. The journal posts when approved.",
                body: form,
                ..Modal::default()
            })?);
        }
        let mut query = self.list.to_params();
        if self.draft.is_some() {
            query.insert("new".into(), "1".into());
        }
        Ok(LiveBody {
            html: shell("ledger-journals", PageVariant::List, &self.flash, page)?,
            query,
            rows: shown,
            ..LiveBody::default()
        })
    }

    async fn table(&mut self, scope: &Scope, units: u8, body: &mut Html) -> Result<usize> {
        let mut input = json!({"page_size": LIMIT});
        if self.filter("status") != "all" {
            input["status"] = json!(self.filter("status"));
        }
        if let Ok(after) = self.filter("after").parse::<i64>() {
            input["after"] = json!(after);
        }
        let form = self.list.filters_form();
        body.push(&render(FilterToolbar::new(
            "ledger-journals-filters",
            vec![ToolbarControl::select(
                "ledger-journals-status",
                "status",
                "Status",
                &form["status"],
                vec![
                    Choice::new("All journals", "all"),
                    Choice::new("Waiting for approval", "submitted"),
                    Choice::new("Posted", "approved"),
                    Choice::new("Rejected", "rejected"),
                ],
            )],
        ))?);
        let display = self.pages.display(scope).await?;
        let records: Vec<view::Journal> = match self
            .pages
            .read(scope, "finance.ledger.journal.list", input)
            .await?
        {
            Read::Found(value) => view::parse(value["records"].clone()).unwrap_or_default(),
            other => {
                body.push(&unavailable(
                    "ledger-journals-refused",
                    "manual journals",
                    &other,
                )?);
                return Ok(0);
            }
        };
        self.columns.load(&records, |j| j.id.to_string());
        let mut rows = Vec::new();
        for journal in &records {
            let total: Decimal = journal.lines.iter().map(|l| view::decimal(&l.debit)).sum();
            let mut row = FlexRow::new(journal.id.to_string())
                .slot(
                    "number",
                    render(RecordLink {
                        workspace: self.framed,
                        kind: crate::MODULE,
                        record_id: &format!("journal-{}", journal.id),
                        href: &format!("{INDEX}/{}", journal.id),
                        class: "font-medium text-ink-strong hover:underline",
                        body: Html::text(&journal.journal_number.to_string()),
                    })?,
                )
                .cell("date", Cell::text(&journal.entry_date))
                .cell("description", Cell::text(&journal.description))
                .cell(
                    "reference",
                    Cell::text(journal.reference.as_deref().unwrap_or("—")),
                )
                .slot("amount", figure(&view::amount(&total.to_string(), units))?)
                .slot("status", status_badge(&journal.status)?)
                .cell(
                    "by",
                    Cell::text(who(
                        journal.submitted_by_user_id,
                        journal.submitted_by_impersonator_id,
                    )),
                )
                .slot(
                    "at",
                    instant(
                        &display,
                        &journal.submitted_at,
                        &format!("ledger-journal-at-{}", journal.id),
                    )?,
                );
            row.dom_id = Some(format!("ledger-journal-{}", journal.id));
            rows.push(row);
        }
        let mut table = FlexTable::new("ledger-journals-table", self.columns.flex_columns(), rows);
        table.caption = "Manual journals";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(Html::text(
            "No journals match. Submit a journal to start one.",
        )));
        let mut card = render(table)?;
        if records.len() == LIMIT {
            if let Some(last) = records.last() {
                let mut later = self.list.clone();
                later
                    .filters
                    .insert("after".into(), last.journal_number.to_string());
                card.push(&render(LaterLink {
                    href: params::address(INDEX, &later.to_params()),
                })?);
            }
        }
        body.push(&render(Card {
            id: "ledger-journals-card",
            inner_class: "p-0",
            body: card,
            ..Card::default()
        })?);
        Ok(records.len())
    }

    fn form(&self, accounts: &[view::Account]) -> Result<Html> {
        let draft = self.draft.as_ref().ok_or(Error::Internal)?;
        let errors = |name: &str| -> Vec<String> {
            draft
                .refusal
                .field(name)
                .iter()
                .map(|m| error_text(name, m))
                .collect()
        };
        let mut choices = vec![];
        choices.extend(
            accounts
                .iter()
                .filter(|a| a.active)
                .map(|a| Choice::new(format!("{} {}", a.code, a.name), a.id.to_string())),
        );
        let mut body = Html::EMPTY;
        let mut messages = draft.refusal.alert(&[
            "entry_date",
            "description",
            "reference",
            "account_id",
            "debit",
            "credit",
        ]);
        if !draft.notice.is_empty() {
            messages.push(draft.notice.clone());
        }
        if !messages.is_empty() {
            body.push(&render(Alert {
                id: "ledger-journal-notice",
                kind: StatusKind::Error,
                class: "mb-3",
                body: Html::text(&messages.join(" ")),
            })?);
        }
        for field in [
            Field::in_form("journal", "entry_date")
                .id("ledger-journal-date")
                .label("Date")
                .kind(InputKind::Date)
                .value(&draft.date)
                .required()
                .errors(errors("entry_date")),
            Field::in_form("journal", "description")
                .id("ledger-journal-description")
                .label("Description")
                .value(&draft.description)
                .required()
                .errors(errors("description"))
                .attr("data-debounce", "300"),
            Field::in_form("journal", "reference")
                .id("ledger-journal-reference")
                .label("Reference")
                .value(&draft.reference)
                .errors(errors("reference"))
                .attr("data-debounce", "300"),
        ] {
            body.push(&render(field)?);
        }
        for (i, line) in draft.lines.iter().enumerate() {
            let mut remove = Html::EMPTY;
            if draft.lines.len() > 2 {
                remove = render(Button {
                    id: &format!("ledger-journal-remove-{i}"),
                    button_type: "button",
                    event: "remove_line",
                    name: "index",
                    value: &i.to_string(),
                    body: Html::text("Remove"),
                    ..Button::default()
                })?;
            }
            let cell = |key: &str, label: &str| {
                Field::in_form("journal", &format!("line{i}_{key}"))
                    .id(format!("ledger-journal-line{i}-{key}"))
                    .label(label)
                    .wrapper_class("mb-0")
                    .errors(draft.line_errors(i, if key == "account" { "account_id" } else { key }))
            };
            let account = render(
                cell("account", "Account")
                    .kind(InputKind::Select)
                    .prompt("Choose an account")
                    .options(choices.clone())
                    .value(&line.account),
            )?;
            let debit = render(
                cell("debit", "Debit")
                    .value(&line.debit)
                    .attr("inputmode", "decimal")
                    .attr("data-debounce", "300"),
            )?;
            let credit = render(
                cell("credit", "Credit")
                    .value(&line.credit)
                    .attr("inputmode", "decimal")
                    .attr("data-debounce", "300"),
            )?;
            let note = render(
                cell("description", "Line description")
                    .value(&line.description)
                    .attr("data-debounce", "300"),
            )?;
            // The ledger names a line problem on the account field when the
            // account is missing, and on the amount fields otherwise.
            body.push(&render(LineRow {
                n: i,
                account,
                debit,
                credit,
                note,
                remove,
            })?);
        }
        let (debits, credits) = draft.totals();
        let difference = debits - credits;
        body.push(&render(Totals {
            text: format!(
                "Debits {debits}. Credits {credits}. {}",
                if difference.is_zero() {
                    "They match.".to_owned()
                } else {
                    format!("The difference is {}.", difference.abs())
                }
            ),
        })?);
        let mut actions = render(Button {
            id: "ledger-journal-add-line",
            button_type: "button",
            event: "add_line",
            disabled: draft.lines.len() >= MAX_LINES,
            body: Html::text("Add line"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "ledger-journal-cancel",
            button_type: "button",
            event: "close_modal",
            body: Html::text("Cancel"),
            ..Button::default()
        })?);
        actions.push(&render(Button {
            id: "ledger-journal-submit",
            button_type: "button",
            event: "submit_journal",
            variant: ButtonVariant::Primary,
            working: "Submitting…",
            body: Html::text("Submit for approval"),
            ..Button::default()
        })?);
        render(Form {
            id: "ledger-journal-form",
            event: "validate",
            change: true,
            class: "mt-4",
            body,
            actions,
        })
    }

    async fn submit(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(draft) = &mut self.draft else {
            return Ok(());
        };
        let input = draft.input();
        match self
            .pages
            .submit(
                r.scope,
                r.credential,
                "submit_journal",
                "finance.ledger.journal.submit",
                input,
            )
            .await?
        {
            Ok(done) => {
                let number = done["result"]["record"]["journal_number"].as_i64();
                self.draft = None;
                self.flash.success(match number {
                    Some(n) => {
                        format!("Journal {n} is submitted and awaiting approval.")
                    }
                    None => "The journal is submitted.".into(),
                });
            }
            Err(Failure::Refused(refusal)) => {
                draft.refusal = refusal;
                draft.notice.clear();
            }
            Err(failure) => draft.notice = failure.text("The journal"),
        }
        Ok(())
    }
}

pub(super) fn status_badge(status: &str) -> Result<Html> {
    render(match status {
        "approved" => Badge::status(BadgeKind::Success, "Posted"),
        "rejected" => Badge::status(BadgeKind::Danger, "Rejected"),
        _ => Badge::status(BadgeKind::Warning, "Submitted"),
    })
}

#[derive(Template)]
#[template(
    source = r#"<div id="ledger-journal-line-{{ n }}" class="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-4"><div class="sm:col-span-2">{{ account|safe }}</div>{{ debit|safe }}{{ credit|safe }}<div class="sm:col-span-2">{{ note|safe }}</div><div>{{ remove|safe }}</div></div>"#,
    ext = "html"
)]
struct LineRow {
    n: usize,
    account: Html,
    debit: Html,
    credit: Html,
    note: Html,
    remove: Html,
}

#[derive(Template)]
#[template(
    source = r#"<p id="ledger-journal-totals" class="mt-3 text-sm text-ink-muted">{{ text }}</p>"#,
    ext = "html"
)]
struct Totals {
    text: String,
}

#[derive(Template)]
#[template(
    source = r#"<div class="p-3 text-sm"><a id="ledger-journals-later" href="{{ href }}" data-patch>Later journals</a></div>"#,
    ext = "html"
)]
struct LaterLink {
    href: String,
}

impl LiveState<Scope, Error> for Journals {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh => {}
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                }
                LiveInput::Event(value) => match event_type(value)? {
                    "filters" => {
                        let mut posted = params::form_values(value, FILTERS_FORM);
                        posted.insert("after".into(), String::new());
                        self.list.apply_filters(&posted);
                    }
                    "grid" => {
                        if let ColumnsOutcome::Sort(_) =
                            self.columns.handle(&params::string_values(value))
                        {
                        }
                    }
                    "new" => {
                        if self.pages.allowed(r.scope, SUBMIT_JOURNALS).await? {
                            let today = self.pages.today(r.scope).await?.to_string();
                            self.draft = Some(Draft::new(today));
                        }
                    }
                    "close_modal" => self.draft = None,
                    "validate" => {
                        if let Some(draft) = &mut self.draft {
                            draft.read(value);
                        }
                    }
                    "add_line" => {
                        if let Some(draft) = &mut self.draft {
                            if draft.lines.len() < MAX_LINES {
                                draft.lines.push(LineDraft::default());
                            }
                        }
                    }
                    "remove_line" => {
                        if let Some(draft) = &mut self.draft {
                            let index = value["index"]
                                .as_str()
                                .and_then(|i| i.parse::<usize>().ok());
                            if let Some(index) =
                                index.filter(|i| *i < draft.lines.len() && draft.lines.len() > 2)
                            {
                                draft.lines.remove(index);
                                draft.refusal = Refusal::default();
                            }
                        }
                    }
                    "submit_journal" => self.submit(&r).await?,
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                },
            }
            self.view(r.scope).await
        })
    }
}
