//! The company's book: its currency and decimal places, opened once, and the
//! two controls kept beside it (the Bookkeeping page changes them).
use super::{
    error_text, event_type, no_company, render, shell, unavailable, view, Failure, Flash, Pages,
    Refusal,
};
use crate::MANAGE_ACCOUNTS;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, Badge, BadgeKind, Button, ButtonVariant, Card, Choice, ConfirmDialog, Fact,
    Facts, Field, Form, Header, Html, InputKind, PageVariant, SectionHeading, StatusKind,
};
use serde_json::json;

pub(super) const INDEX: &str = "/finance/ledger/book";

pub(super) struct Book {
    pages: Pages,
    currency: String,
    units: String,
    approver: String,
    approval_reason: String,
    refusal: Refusal,
    notice: String,
    confirming: bool,
    flash: Flash,
}

impl Book {
    pub(super) fn new(pages: Pages) -> Self {
        Self {
            pages,
            currency: String::new(),
            units: "2".into(),
            approver: "someone_else".into(),
            approval_reason: String::new(),
            refusal: Refusal::default(),
            notice: String::new(),
            confirming: false,
            flash: Flash::default(),
        }
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let mut body = Html::EMPTY;
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-book-no-company")?);
        } else {
            match self.pages.book(scope).await? {
                Err(refused) => {
                    body.push(&unavailable("ledger-book-refused", "the book", &refused)?)
                }
                Ok(state) => match &state.book {
                    Some(book) => body.push(&self.facts(&state, book)?),
                    None => body.push(&self.open_form(scope).await?),
                },
            }
        }
        let mut page = render(Header {
            title: Html::text("Book"),
            ..Header::default()
        })?;
        page.push(&body);
        Ok(LiveBody {
            html: shell("ledger-book", PageVariant::Detail, &self.flash, page)?,
            ..LiveBody::default()
        })
    }

    fn facts(&self, state: &view::BookState, book: &view::BookRow) -> Result<Html> {
        let controlled = state
            .controlled_bookkeeping
            .as_ref()
            .and_then(|c| c.controlled)
            .unwrap_or(true);
        let required = state
            .duty_separation
            .as_ref()
            .and_then(|c| c.required)
            .unwrap_or(true);
        let yes_no = |on: bool, yes: &str, no: &str| -> Result<Html> {
            render(Badge::status(
                if on {
                    BadgeKind::Success
                } else {
                    BadgeKind::Warning
                },
                if on { yes } else { no },
            ))
        };
        let mut body = render(SectionHeading {
            id: "ledger-book-heading",
            title: "Currency",
            description: "Chosen when the book was opened. It cannot be changed.",
            ..SectionHeading::default()
        })?;
        body.push(&render(Facts {
            id: "ledger-book-facts",
            items: vec![
                Fact {
                    title: "Currency",
                    id: "ledger-book-currency",
                    value: Html::text(&book.currency_code),
                },
                Fact {
                    title: "Decimal places",
                    id: "ledger-book-units",
                    value: Html::text(&book.minor_units.to_string()),
                },
            ],
        })?);
        let mut controls = render(SectionHeading {
            id: "ledger-book-controls-heading",
            title: "Controls",
            description: "Controlled bookkeeping restricts posting to open periods. Separation of duties requires independent approval. Manage these on Bookkeeping and Approval exceptions.",
            ..SectionHeading::default()
        })?;
        controls.push(&render(Facts {
            id: "ledger-book-controls",
            items: vec![
                Fact {
                    title: "Controlled bookkeeping",
                    id: "ledger-book-controlled",
                    value: yes_no(controlled, "On", "Off")?,
                },
                Fact {
                    title: "Separation of duties",
                    id: "ledger-book-separation",
                    value: yes_no(required, "Required", "Not required")?,
                },
            ],
        })?);
        controls.push(&render(Button {
            id: "ledger-book-approval-settings",
            href: "/finance/reports/approval-exceptions",
            body: Html::text("Approval settings"),
            ..Button::default()
        })?);
        let mut html = render(Card {
            id: "ledger-book-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-book-heading",
            body,
            ..Card::default()
        })?;
        html.push(&render(Card {
            id: "ledger-book-controls-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-book-controls-heading",
            class: "mt-4",
            body: controls,
            ..Card::default()
        })?);
        Ok(html)
    }

    async fn open_form(&self, scope: &Scope) -> Result<Html> {
        if !self.pages.allowed(scope, MANAGE_ACCOUNTS).await? {
            return render(
                bilibi_ui::EmptyState::missing(
                    "The book is not open yet",
                    "Ask someone who manages the chart of accounts to open it.",
                )
                .id("ledger-book-not-open"),
            );
        }
        let errors = |name: &str| -> Vec<String> {
            self.refusal
                .field(name)
                .iter()
                .map(|m| error_text(name, m))
                .collect()
        };
        let mut body = Html::EMPTY;
        let mut messages = self.refusal.alert(&["currency_code", "minor_units"]);
        if !self.notice.is_empty() {
            messages.push(self.notice.clone());
        }
        if !messages.is_empty() {
            body.push(&render(Alert {
                id: "ledger-book-notice",
                kind: StatusKind::Error,
                body: Html::text(&messages.join(" ")),
                ..Alert::default()
            })?);
        }
        body.push(&render(
            Field::in_form("book", "currency_code")
                .id("ledger-book-currency-code")
                .label("Currency code")
                .value(&self.currency)
                .required()
                .hint("Three letters, like MYR or USD. You cannot change it later.")
                .errors(errors("currency_code"))
                .attr("maxlength", "3")
                .attr("data-debounce", "300"),
        )?);
        body.push(&render(
            Field::in_form("book", "minor_units")
                .id("ledger-book-minor-units")
                .label("Decimal places")
                .value(&self.units)
                .kind(InputKind::Select)
                .options((0..=4).map(|n| Choice::new(n.to_string(), n.to_string())).collect())
                .hint("How many digits follow the point in an amount: 2 for 12.50. You cannot change it later.")
                .errors(errors("minor_units")),
        )?);
        body.push(&render(
            Field::in_form("book", "approver")
                .id("ledger-book-approver")
                .label("Approval policy")
                .kind(InputKind::Select)
                .value(&self.approver)
                .options(vec![
                    Choice::new("Independent approval required", "someone_else"),
                    Choice::new("Self-approval permitted", "one_person"),
                ])
                .hint("Approving your own work records an exception for an independent reviewer."),
        )?);
        if self.approver == "one_person" {
            body.push(&render(
                Field::in_form("book", "approval_reason")
                    .id("ledger-book-approval-reason")
                    .label("Reason for allowing self-approval")
                    .value(&self.approval_reason)
                    .required()
                    .errors(errors("reason")),
            )?);
        }
        let actions = render(Button {
            id: "ledger-book-open",
            button_type: "button",
            event: "review_open",
            variant: ButtonVariant::Primary,
            body: Html::text("Open the book"),
            ..Button::default()
        })?;
        let mut card = render(SectionHeading {
            id: "ledger-book-open-heading",
            title: "Open the book",
            ..SectionHeading::default()
        })?;
        card.push(&render(Form {
            id: "ledger-book-form",
            event: "validate",
            change: true,
            class: "mt-4 max-w-md",
            body,
            actions,
        })?);
        let mut html = render(Card {
            id: "ledger-book-open-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-book-open-heading",
            body: card,
            ..Card::default()
        })?;
        if self.confirming {
            html.push(&render(ConfirmDialog {
                id: "ledger-book-confirm",
                consequence: &format!(
                    "Open the book in {} with {} decimal places?",
                    self.currency.trim().to_uppercase(),
                    self.units
                ),
                detail: if self.approver == "one_person" { "The currency and decimal places are permanent. You may approve your own work; each approval is recorded in Approval exceptions." } else { "The currency and decimal places are permanent. Someone else must approve your work." },
                confirm: "Open the book",
                working: "Opening…",
                confirm_event: "open_book",
                cancel_event: "cancel_open",
            })?);
        }
        Ok(html)
    }
}

impl LiveState<Scope, Error> for Book {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(value) = r.input {
                match event_type(value)? {
                    "validate" => {
                        let fields = params::form_values(value, "book");
                        self.currency = fields.get("currency_code").cloned().unwrap_or_default();
                        self.units = fields
                            .get("minor_units")
                            .cloned()
                            .unwrap_or_else(|| "2".into());
                        self.approver = fields
                            .get("approver")
                            .cloned()
                            .unwrap_or_else(|| "someone_else".into());
                        if let Some(reason) = fields.get("approval_reason") {
                            self.approval_reason = reason.clone();
                        }
                        self.refusal = Refusal::default();
                        self.notice.clear();
                    }
                    "review_open" => {
                        self.refusal = Refusal::default();
                        self.confirming = self.currency.trim().len() == 3 || {
                            self.refusal.fields.insert(
                                "currency_code".into(),
                                vec!["must be three letters".into()],
                            );
                            false
                        };
                    }
                    "cancel_open" => self.confirming = false,
                    "open_book" => {
                        self.confirming = false;
                        let units: i64 = self.units.parse().unwrap_or(-1);
                        match self
                            .pages
                            .submit(
                                r.scope,
                                r.credential,
                                "open_book",
                                "finance.ledger.book.open",
                                json!({"currency_code": self.currency.trim().to_uppercase(), "minor_units": units,"separation_required":self.approver != "one_person","approval_reason": self.approval_reason}),
                            )
                            .await?
                        {
                            Ok(_) => self.flash.success("The book is open."),
                            Err(Failure::Refused(refusal)) => self.refusal = refusal,
                            Err(failure) => self.notice = failure.text("The book"),
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
