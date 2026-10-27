//! Fiscal years and their monthly periods: open a year, then close, reopen and
//! lock its periods, and close or reopen the year. Each action calls the same
//! contract an agent calls; the ledger decides what is allowed and says why not.
use super::{
    capability_for, dialog_actions, error_text, event_type, no_company, render, shell, unavailable,
    view, Failure, Flash, Pages, Read, Refusal, Review,
};
use crate::MANAGE_PERIODS;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Card, Cell,
    ConfirmDialog, CreateButton, EmptyState, Field, FlexRow, FlexTable, Form, Header, Html,
    InputKind, ListColumns, Modal, PageVariant, SectionHeading, StatusKind, TableEmpty,
};
use jiff::{civil::Date, ToSpan};
use serde_json::json;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    ClosePeriod,
    LockPeriod,
    ReopenPeriod,
    CloseYear,
    LockYear,
    ReopenYear,
}

impl Kind {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "close_period" => Self::ClosePeriod,
            "lock_period" => Self::LockPeriod,
            "reopen_period" => Self::ReopenPeriod,
            "close_year" => Self::CloseYear,
            "lock_year" => Self::LockYear,
            "reopen_year" => Self::ReopenYear,
            _ => return None,
        })
    }
    /// The page event and the contract it calls.
    fn event(self) -> &'static str {
        match self {
            Self::ClosePeriod => "close_period",
            Self::LockPeriod => "lock_period",
            Self::ReopenPeriod => "reopen_period",
            Self::CloseYear => "close_year",
            Self::LockYear => "lock_year",
            Self::ReopenYear => "reopen_year",
        }
    }
    fn contract(self) -> &'static str {
        match self {
            Self::ClosePeriod => "finance.ledger.period.close",
            Self::LockPeriod => "finance.ledger.period.lock",
            Self::ReopenPeriod => "finance.ledger.period.reopen",
            Self::CloseYear => "finance.ledger.fiscal_year.close",
            Self::LockYear => "finance.ledger.fiscal_year.lock",
            Self::ReopenYear => "finance.ledger.fiscal_year.reopen",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::ClosePeriod | Self::LockPeriod | Self::ReopenPeriod => "period_id",
            _ => "fiscal_year_id",
        }
    }
    fn needs_reason(self) -> bool {
        matches!(self, Self::ReopenPeriod | Self::ReopenYear)
    }
}

/// The one period each period action can reach now, in the ledger's own
/// order: close the earliest open one, reopen the latest closed one, lock
/// the earliest unlocked one.
#[derive(Clone, Copy)]
struct Targets {
    close: Option<i64>,
    reopen: Option<i64>,
    lock: Option<i64>,
    can_close: bool,
    can_reopen: bool,
    can_lock: bool,
    can_post: bool,
}

/// What the person chose, and the ledger's review of it once it has one.
struct Choice {
    kind: Kind,
    company: Option<i64>,
    id: i64,
    label: String,
    review: Option<Review>,
    reason: String,
    refusal: Refusal,
    notice: String,
}

#[derive(Default)]
struct Opening {
    start: String,
    end: String,
    calculated_end: String,
    review: Option<Review>,
    refusal: Refusal,
    notice: String,
}

pub(super) struct Years {
    pages: Pages,
    flash: Flash,
    columns: Option<ListColumns>,
    choice: Option<Choice>,
    opening: Option<Opening>,
}

impl Years {
    pub(super) fn new(pages: Pages) -> Self {
        Self {
            pages,
            flash: Flash::default(),
            columns: None,
            choice: None,
            opening: None,
        }
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let company_name = if let Some(company) = scope.actor().organisation() {
            let client = self.pages.db.get().await?;
            self.pages
                .parties
                .organisation_name(&**client, scope, company)
                .await?
                .map(|company| company.name)
                .unwrap_or_else(|| "Company unavailable".into())
        } else {
            "Company unavailable".into()
        };
        let mut body = Html::EMPTY;
        let mut actions = Html::EMPTY;
        let mut shown = 0;
        let can_open = self.pages.allowed(scope, MANAGE_PERIODS).await?;
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-years-no-company")?);
        } else {
            match self.pages.book(scope).await? {
                Err(refused) => body.push(&unavailable(
                    "ledger-years-refused",
                    "fiscal years",
                    &refused,
                )?),
                Ok(state) if state.book.is_none() => body.push(&render(
                    EmptyState::missing(
                        "The book is not open yet",
                        "Open the company's book first. Then open its first fiscal year.",
                    )
                    .id("ledger-years-no-book"),
                )?),
                Ok(_) => {
                    if can_open {
                        actions.push(&render(CreateButton {
                            noun: "Fiscal year",
                            id: "ledger-year-new",
                            class: "text-xs",
                            event: "open_dialog",
                            button_type: "button",
                            ..CreateButton::default()
                        })?);
                    }
                    match self
                        .pages
                        .read(
                            scope,
                            "finance.ledger.fiscal_year.list",
                            json!({"page_size": 100}),
                        )
                        .await?
                    {
                        Read::Found(value) => {
                            let years: Vec<view::Year> =
                                view::parse(value["records"].clone()).unwrap_or_default();
                            shown = years.len();
                            if years.is_empty() {
                                body.push(&render(
                                    EmptyState::missing(
                                        "No fiscal year yet",
                                        "A company posts only into a period of an open fiscal year. Open the first one.",
                                    )
                                    .id("ledger-years-empty"),
                                )?);
                            }
                            // The ledger reopens the latest closed period and locks
                            // the earliest unlocked one; offer only those.
                            let all: Vec<&view::Period> =
                                years.iter().flat_map(|y| y.periods.iter()).collect();
                            let reopen = all
                                .iter()
                                .rposition(|p| p.status == "closed")
                                .filter(|i| all.get(i + 1).is_none_or(|p| p.status == "open"))
                                .map(|i| all[i].id);
                            let lock = all
                                .iter()
                                .position(|p| p.status != "locked")
                                .filter(|i| all[*i].status == "closed")
                                .map(|i| all[i].id);
                            let close = all.iter().find(|p| p.status == "open").map(|p| p.id);
                            let can_close = self
                                .pages
                                .allowed(scope, capability_for("close_period"))
                                .await?;
                            let can_reopen = self
                                .pages
                                .allowed(scope, capability_for("reopen_period"))
                                .await?;
                            let can_lock = self
                                .pages
                                .allowed(scope, capability_for("lock_period"))
                                .await?;
                            let can_post = self.pages.allowed(scope, crate::POST_ENTRIES).await?;
                            for year in years.iter().rev() {
                                body.push(&self.year(
                                    year,
                                    &company_name,
                                    Targets {
                                        close,
                                        reopen,
                                        lock,
                                        can_close,
                                        can_reopen,
                                        can_lock,
                                        can_post,
                                    },
                                )?);
                            }
                        }
                        other => body.push(&unavailable(
                            "ledger-years-refused",
                            "fiscal years",
                            &other,
                        )?),
                    }
                }
            }
        }
        let mut page = render(Header {
            title: Html::text("Fiscal years"),
            subtitle: &company_name,
            actions,
            ..Header::default()
        })?;
        page.push(&body);
        if let Some(opening) = &self.opening {
            if can_open {
                page.push(&render(Modal {
                    id: "ledger-year-modal",
                    title: &format!("Open a fiscal year for {company_name}"),
                    cancel_event: "close_dialog",
                    description: "A year starts on the first of a month and ends on the last day of a month. Later years follow the one before.",
                    body: self.open_form(opening)?,
                    ..Modal::default()
                })?);
                if let Some(review) = &opening.review {
                    page.push(&render(ConfirmDialog {
                        id: "ledger-year-confirm",
                        consequence: &format!("Open fiscal year for {company_name}?"),
                        detail: &format!(
                            "{} to {}",
                            review.facts["start_date"].as_str().unwrap_or(""),
                            review.facts["end_date"].as_str().unwrap_or("")
                        ),
                        confirm: "Open fiscal year",
                        working: "Opening…",
                        confirm_event: "confirm_open_year",
                        cancel_event: "cancel_open_review",
                    })?);
                }
            }
        }
        if let Some(choice) = &self.choice {
            page.push(&self.dialog(choice, &company_name)?);
        }
        Ok(LiveBody {
            html: shell("ledger-years", PageVariant::List, &self.flash, page)?,
            rows: shown,
            ..LiveBody::default()
        })
    }

    fn year(&mut self, year: &view::Year, company_name: &str, targets: Targets) -> Result<Html> {
        let Targets {
            close: close_id,
            reopen: reopen_id,
            lock: lock_id,
            can_close: manage,
            can_reopen: reopen,
            can_lock: lock,
            can_post: post,
        } = targets;
        let closed = year.status == "closed";
        let all_locked = year.periods.iter().all(|p| p.status == "locked");
        if self.columns.is_none() {
            self.columns = Some(
                ListColumns::mount(
                    "ledger-year-periods",
                    vec![
                        BuiltinColumn::new("number", "Period"),
                        BuiltinColumn::new("start", "From"),
                        BuiltinColumn::new("end", "To"),
                        BuiltinColumn::new("status", "Status"),
                    ],
                )
                .map_err(|_| Error::Internal)?,
            );
        }
        let columns = self.columns.as_mut().ok_or(Error::Internal)?;
        columns.load(&year.periods, |p| p.id.to_string());
        let mut rows = Vec::new();
        let last = year.periods.last().map(|p| p.id);
        for period in &year.periods {
            let label = format!(
                "period {} ({} to {})",
                period.period_number, period.start_date, period.end_date
            );
            let mut actions = Html::EMPTY;
            let mut button =
                |kind: &str, text: &'static str, variant: ButtonVariant| -> Result<()> {
                    actions.push(&render(Button {
                        id: &format!("ledger-period-{kind}-{}", period.id),
                        button_type: "button",
                        event: "choose",
                        name: "action",
                        value: &format!("{kind}:{}:{}", period.id, label),
                        variant,
                        body: Html::text(text),
                        ..Button::default()
                    })?);
                    Ok(())
                };
            match period.status.as_str() {
                // The last period of a year closes with the year.
                "open" if manage && close_id == Some(period.id) && Some(period.id) != last => {
                    button("close_period", "Close", ButtonVariant::Secondary)?
                }
                "closed" => {
                    if reopen && !closed && reopen_id == Some(period.id) {
                        button("reopen_period", "Reopen", ButtonVariant::Secondary)?;
                    }
                    if lock && lock_id == Some(period.id) {
                        button("lock_period", "Lock", ButtonVariant::Danger)?;
                    }
                }
                _ => {}
            }
            let mut row = FlexRow::new(period.id.to_string())
                .cell("number", Cell::text(period.period_number.to_string()))
                .cell("start", Cell::text(&period.start_date))
                .cell("end", Cell::text(&period.end_date))
                .slot("status", period_badge(&period.status)?);
            row.actions = actions;
            row.dom_id = Some(format!("ledger-period-{}", period.id));
            rows.push(row);
        }
        let table_id = format!("ledger-year-{}-periods", year.id);
        let mut table = FlexTable::new(&table_id, columns.flex_columns(), rows);
        table.caption = "Periods";
        table.framed = false;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.actions = true;
        table.empty = Some(TableEmpty::Content(Html::text("This year has no periods.")));

        let mut year_actions = Html::EMPTY;
        let mine = |id: Option<i64>| id.is_some_and(|id| year.periods.iter().any(|p| p.id == id));
        if !closed && manage && post && mine(close_id) {
            year_actions.push(&render(Button {
                id: &format!("ledger-year-close-{}", year.id),
                button_type: "button",
                event: "choose",
                name: "action",
                value: &format!(
                    "close_year:{}:{} to {}",
                    year.id, year.start_date, year.end_date
                ),
                variant: ButtonVariant::Primary,
                body: Html::text("Close year"),
                ..Button::default()
            })?);
        }
        if closed && reopen && post && !all_locked && reopen_id == year.periods.last().map(|p| p.id)
        {
            year_actions.push(&render(Button {
                id: &format!("ledger-year-reopen-{}", year.id),
                button_type: "button",
                event: "choose",
                name: "action",
                value: &format!(
                    "reopen_year:{}:{} to {}",
                    year.id, year.start_date, year.end_date
                ),
                body: Html::text("Reopen year"),
                ..Button::default()
            })?);
        }
        if closed && lock && !all_locked && mine(lock_id) {
            year_actions.push(&render(Button {
                id: &format!("ledger-year-lock-{}", year.id),
                button_type: "button",
                event: "choose",
                name: "action",
                value: &format!(
                    "lock_year:{}:{} to {}",
                    year.id, year.start_date, year.end_date
                ),
                variant: ButtonVariant::Danger,
                body: Html::text("Lock year"),
                ..Button::default()
            })?);
        }
        let heading_id = format!("ledger-year-{}-heading", year.id);
        let mut heading = render(SectionHeading {
            id: &heading_id,
            title: &format!(
                "{} · {} to {}",
                company_name, year.start_date, year.end_date
            ),
            title_actions: render(Badge::status(
                if closed {
                    BadgeKind::Warning
                } else {
                    BadgeKind::Success
                },
                if closed { "Closed" } else { "Open" },
            ))?,
            actions: year_actions,
            class: "p-5 pb-0",
            ..SectionHeading::default()
        })?;
        heading.push(&render(table)?);
        render(Card {
            id: &format!("ledger-year-{}", year.id),
            inner_class: "p-0",
            class: "mt-4",
            role: "region",
            labelledby: &heading_id,
            body: heading,
            ..Card::default()
        })
    }

    fn open_form(&self, opening: &Opening) -> Result<Html> {
        let errors = |name: &str| -> Vec<String> {
            opening
                .refusal
                .field(name)
                .iter()
                .map(|m| error_text(name, m))
                .collect()
        };
        let mut body = Html::EMPTY;
        let mut messages = opening.refusal.alert(&["start_date", "end_date"]);
        if !opening.notice.is_empty() {
            messages.push(opening.notice.clone());
        }
        if !messages.is_empty() {
            body.push(&render(Alert {
                id: "ledger-year-notice",
                kind: StatusKind::Error,
                class: "mb-3",
                body: Html::text(&messages.join(" ")),
            })?);
        }
        body.push(&render(
            Field::in_form("year", "start_date")
                .id("ledger-year-start")
                .label("First day")
                .kind(InputKind::Date)
                .value(&opening.start)
                .hint("Start on the first of a month. The first year requires a date; later years default to the day after the previous year ends.")
                .errors(errors("start_date")),
        )?);
        body.push(&render(
            Field::in_form("year", "end_date")
                .id("ledger-year-end")
                .label("Last day")
                .kind(InputKind::Date)
                .value(&opening.end)
                .hint(end_hint(opening))
                .errors(errors("end_date")),
        )?);
        let mut actions = render(Button {
            id: "ledger-year-cancel",
            button_type: "button",
            event: "close_dialog",
            body: Html::text("Cancel"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "ledger-year-open",
            button_type: "button",
            event: "open_year",
            variant: ButtonVariant::Primary,
            working: "Opening…",
            body: Html::text("Open fiscal year"),
            ..Button::default()
        })?);
        render(Form {
            id: "ledger-year-form",
            event: "validate",
            change: true,
            class: "mt-4",
            body,
            actions: dialog_actions(actions)?,
        })
    }

    fn dialog(&self, choice: &Choice, company_name: &str) -> Result<Html> {
        if choice.kind.needs_reason() {
            let errors: Vec<String> = choice
                .refusal
                .field("reason")
                .iter()
                .map(|m| error_text("reason", m))
                .collect();
            let mut body = Html::EMPTY;
            let mut messages = choice.refusal.alert(&["reason"]);
            if !choice.notice.is_empty() {
                messages.push(choice.notice.clone());
            }
            if !messages.is_empty() {
                body.push(&render(Alert {
                    id: "ledger-choice-notice",
                    kind: StatusKind::Error,
                    class: "mb-3",
                    body: Html::text(&messages.join(" ")),
                })?);
            }
            body.push(&render(
                Field::in_form("reopen", "reason")
                    .id("ledger-reopen-reason")
                    .label("Reason")
                    .kind(InputKind::Textarea)
                    .value(&choice.reason)
                    .required()
                    .hint("Up to 500 characters. The ledger records the reopening for an independent reviewer.")
                    .errors(errors)
                    .attr("data-debounce", "300"),
            )?);
            let mut actions = render(Button {
                id: "ledger-reopen-cancel",
                button_type: "button",
                event: "cancel_choice",
                body: Html::text("Cancel"),
                ..Button::default()
            })?;
            actions.push(&render(Button {
                id: "ledger-reopen-submit",
                button_type: "button",
                event: "do_reopen",
                variant: ButtonVariant::Primary,
                working: "Reopening…",
                body: Html::text("Reopen"),
                ..Button::default()
            })?);
            return render(Modal {
                id: "ledger-choice-modal",
                title: &format!(
                    "Reopen {}",
                    if choice.kind == Kind::ReopenYear {
                        "fiscal year"
                    } else {
                        "period"
                    }
                ),
                cancel_event: "cancel_choice",
                description: &format!("{} · {}", company_name, choice.label),
                body: render(Form {
                    id: "ledger-reopen-form",
                    event: "validate",
                    change: true,
                    class: "mt-4",
                    body,
                    actions: dialog_actions(actions)?,
                })?,
                ..Modal::default()
            });
        }
        let (consequence, detail, confirm, working) = match choice.kind {
            Kind::ClosePeriod => (
                format!("Close {}?", choice.label),
                "Nobody can post into it until it is reopened. Periods close in date order."
                    .to_owned(),
                "Close period",
                "Closing…",
            ),
            Kind::LockPeriod => (
                format!("Lock {} for good?", choice.label),
                "A locked period can never be reopened. Periods lock in date order.".to_owned(),
                "Lock period",
                "Locking…",
            ),
            Kind::CloseYear => (
                format!("Close fiscal year {}?", choice.label),
                close_year_detail(choice.review.as_ref()),
                "Close year",
                "Closing…",
            ),
            _ => (
                format!("Lock fiscal year {} for good?", choice.label),
                "Every closed period of the year is locked and can never be reopened.".to_owned(),
                "Lock year",
                "Locking…",
            ),
        };
        render(ConfirmDialog {
            id: "ledger-choice-confirm",
            consequence: &format!("{consequence} ({company_name})"),
            detail: &detail,
            confirm,
            working,
            confirm_event: "do_confirm",
            cancel_event: "cancel_choice",
        })
    }

    async fn choose(&mut self, r: &LiveRequest<'_, Scope>, action: &str) -> Result<()> {
        let mut parts = action.splitn(3, ':');
        let kind = parts.next().and_then(Kind::parse).ok_or(Error::Invalid)?;
        let id: i64 = parts
            .next()
            .and_then(|i| i.parse().ok())
            .ok_or(Error::Invalid)?;
        let label = parts.next().unwrap_or_default().to_owned();
        if !self
            .pages
            .allowed(r.scope, capability_for(kind.event()))
            .await?
        {
            self.flash.error(super::DENIED);
            return Ok(());
        }
        let mut choice = Choice {
            kind,
            company: Pages::company(r.scope),
            id,
            label,
            review: None,
            reason: String::new(),
            refusal: Refusal::default(),
            notice: String::new(),
        };
        if !kind.needs_reason() {
            match self
                .pages
                .review(
                    r.scope,
                    r.credential,
                    "choose",
                    kind.contract(),
                    json!({ kind.key(): id }),
                )
                .await?
            {
                Ok(review) => choice.review = Some(review),
                Err(failure) => {
                    self.flash.error(failure.text("The period"));
                    return Ok(());
                }
            }
        }
        self.choice = Some(choice);
        Ok(())
    }

    async fn confirm(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(choice) = self.choice.take() else {
            return Ok(());
        };
        if choice.company != Pages::company(r.scope) {
            self.flash
                .error("The company changed. Choose the action again.");
            return Ok(());
        }
        let Some(review) = choice.review else {
            return Ok(());
        };
        match self
            .pages
            .commit(
                r.scope,
                r.credential,
                "do_confirm",
                choice.kind.contract(),
                review,
            )
            .await?
        {
            Ok(_) => self.flash.success(match choice.kind {
                Kind::ClosePeriod => "The period is closed.",
                Kind::LockPeriod => "The period is locked for good.",
                Kind::CloseYear => "The fiscal year is closed.",
                _ => "The fiscal year is locked for good.",
            }),
            Err(failure) => self.flash.error(failure.text("The period")),
        }
        Ok(())
    }

    async fn reopen(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(choice) = &mut self.choice else {
            return Ok(());
        };
        if choice.company != Pages::company(r.scope) {
            choice.notice = "The company changed. Choose the action again.".into();
            return Ok(());
        }
        let input = json!({ choice.kind.key(): choice.id, "reason": choice.reason.trim() });
        match self
            .pages
            .submit(
                r.scope,
                r.credential,
                "do_reopen",
                choice.kind.contract(),
                input,
            )
            .await?
        {
            Ok(_) => {
                let year = choice.kind == Kind::ReopenYear;
                self.choice = None;
                self.flash.success(if year {
                    "The fiscal year is reopened. Its last period is open again."
                } else {
                    "The period is open again."
                });
            }
            Err(Failure::Refused(refusal)) => {
                choice.refusal = refusal;
                choice.notice.clear();
            }
            Err(failure) => choice.notice = failure.text("The reopening"),
        }
        Ok(())
    }

    async fn open_year(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(opening) = &mut self.opening else {
            return Ok(());
        };
        let mut input = json!({});
        if !opening.start.trim().is_empty() {
            input["start_date"] = json!(opening.start.trim());
        }
        if !opening.end.trim().is_empty() {
            input["end_date"] = json!(opening.end.trim());
        }
        match self
            .pages
            .review(
                r.scope,
                r.credential,
                "open_year",
                "finance.ledger.fiscal_year.open",
                input,
            )
            .await?
        {
            Ok(review) => opening.review = Some(review),
            Err(Failure::Refused(refusal)) => {
                opening.refusal = refusal;
                opening.notice.clear();
            }
            Err(failure) => opening.notice = failure.text("The fiscal year"),
        }

        Ok(())
    }
    async fn confirm_open_year(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(opening) = &mut self.opening else {
            return Ok(());
        };
        let Some(review) = opening.review.take() else {
            return Ok(());
        };
        if review.input["company_id"].as_i64() != Pages::company(r.scope) {
            opening.notice = "The company changed. Review the fiscal year again.".into();
            return Ok(());
        }
        match self
            .pages
            .commit(
                r.scope,
                r.credential,
                "confirm_open_year",
                "finance.ledger.fiscal_year.open",
                review,
            )
            .await?
        {
            Ok(_) => {
                self.opening = None;
                self.flash.success("The fiscal year is open.");
            }
            Err(Failure::Refused(refusal)) => {
                opening.refusal = refusal;
                opening.notice.clear();
            }
            Err(failure) => opening.notice = failure.text("The fiscal year"),
        }
        Ok(())
    }
}

fn end_hint(opening: &Opening) -> String {
    if opening.end.trim().is_empty() && !opening.calculated_end.is_empty() {
        format!(
            "Left empty, the twelve-month default applies: {}. You may choose another month-end.",
            opening.calculated_end
        )
    } else if opening.end == opening.calculated_end {
        "Calculated as twelve months from the first day. You may choose another month-end.".into()
    } else {
        "Custom end date. Choose the last day of a month.".into()
    }
}

fn default_end(start: &str) -> String {
    start
        .parse::<Date>()
        .ok()
        .and_then(|date| date.checked_add(12.months()).ok())
        .and_then(|date| date.yesterday().ok())
        .map(|date| date.to_string())
        .unwrap_or_default()
}

fn close_year_detail(review: Option<&Review>) -> String {
    let base = "Any open periods close with it, and nobody can post into the year again until it is reopened.";
    let entry = review.map(|r| &r.facts["entry"]).filter(|e| !e.is_null());
    match entry.and_then(|e| e["lines"].as_array()).map(Vec::len) {
        Some(lines) => format!(
            "{base} The ledger also posts one closing entry of {lines} lines that moves income and expenses into retained earnings."
        ),
        None => format!("{base} Nothing is posted: the year has no income or expense to move."),
    }
}

fn period_badge(status: &str) -> Result<Html> {
    render(match status {
        "open" => Badge::status(BadgeKind::Success, "Open"),
        "closed" => Badge::status(BadgeKind::Warning, "Closed"),
        _ => Badge::status(BadgeKind::Neutral, "Locked"),
    })
}

impl LiveState<Scope, Error> for Years {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(value) = r.input {
                match event_type(value)? {
                    "open_dialog" => {
                        if self.pages.allowed(r.scope, MANAGE_PERIODS).await? {
                            let mut opening = Opening::default();
                            let mut after: Option<String> = None;
                            loop {
                                let mut input = json!({"page_size": 100});
                                if let Some(date) = &after {
                                    input["after_start_date"] = json!(date);
                                }
                                let Read::Found(value) = self
                                    .pages
                                    .read(r.scope, "finance.ledger.fiscal_year.list", input)
                                    .await?
                                else {
                                    break;
                                };
                                let years: Vec<view::Year> =
                                    view::parse(value["records"].clone()).unwrap_or_default();
                                if let Some(start) = years
                                    .last()
                                    .and_then(|year| year.end_date.parse::<Date>().ok())
                                    .and_then(|end| end.tomorrow().ok())
                                {
                                    opening.start = start.to_string();
                                    opening.calculated_end = default_end(&opening.start);
                                    opening.end = opening.calculated_end.clone();
                                }
                                after = value["next_after_start_date"].as_str().map(str::to_owned);
                                if after.is_none() {
                                    break;
                                }
                            }
                            self.opening = Some(opening);
                        }
                    }
                    "close_dialog" => self.opening = None,
                    "validate" => {
                        if let Some(opening) = &mut self.opening {
                            let fields = params::form_values(value, "year");
                            let start = fields.get("start_date").cloned().unwrap_or_default();
                            let end = fields.get("end_date").cloned().unwrap_or_default();
                            let follows_default = end == opening.calculated_end;
                            opening.start = start;
                            opening.calculated_end = default_end(&opening.start);
                            opening.end = if follows_default {
                                opening.calculated_end.clone()
                            } else {
                                end
                            };
                            opening.review = None;
                            opening.refusal = Refusal::default();
                            opening.notice.clear();
                        } else if let Some(choice) = &mut self.choice {
                            choice.reason = params::form_values(value, "reopen")
                                .get("reason")
                                .cloned()
                                .unwrap_or_default();
                            choice.refusal = Refusal::default();
                            choice.notice.clear();
                        }
                    }
                    "open_year" => self.open_year(&r).await?,
                    "confirm_open_year" => self.confirm_open_year(&r).await?,
                    "cancel_open_review" => {
                        if let Some(opening) = &mut self.opening {
                            opening.review = None;
                        }
                    }
                    "choose" => {
                        let action = value["action"].as_str().ok_or(Error::Invalid)?.to_owned();
                        self.choose(&r, &action).await?
                    }
                    "cancel_choice" => self.choice = None,
                    "do_confirm" => self.confirm(&r).await?,
                    "do_reopen" => self.reopen(&r).await?,
                    "grid" => {
                        if let Some(columns) = &mut self.columns {
                            columns.handle(&params::string_values(value));
                        }
                    }
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                }
            }
            self.view(r.scope).await
        })
    }
}
