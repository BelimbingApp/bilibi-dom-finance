//! One manual journal, read-only. While it waits, someone with the approval
//! permission approves it (and it posts) or rejects it with a reason. The
//! ledger refuses the person who submitted it when the company separates duties.
use super::{
    dialog_actions, error_text, event_type, figure, instant,
    journals::{status_badge, INDEX},
    no_company, render, shell, unavailable, view, who, Announce, EntryLink, Failure, Flash, Pages,
    Read, Refusal, Review,
};
use crate::APPROVE_JOURNALS;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Alert, BackLink, BuiltinColumn, Button, ButtonVariant, Card, Cell, ConfirmDialog,
    EmptyState, Fact, Facts, Field, FlexRow, FlexTable, Form, Header, Html, InputKind, ListColumns,
    Modal, PageVariant, SectionHeading, StatusKind, TableEmpty,
};
use rust_decimal::Decimal;
use serde_json::json;

#[derive(Default)]
struct Rejecting {
    reason: String,
    refusal: Refusal,
    notice: String,
}

pub(super) struct Journal {
    pages: Pages,
    id: i64,
    flash: Flash,
    lines: Option<ListColumns>,
    approving: Option<Review>,
    rejecting: Option<Rejecting>,
}

impl Journal {
    pub(super) fn new(pages: Pages, id: i64) -> Self {
        Self {
            pages,
            id,
            flash: Flash::default(),
            lines: None,
            approving: None,
            rejecting: None,
        }
    }

    fn simple(&self, title: &str, body: Html) -> Result<LiveBody> {
        let mut page = render(Header {
            title: Html::text(title),
            ..Header::default()
        })?;
        page.push(&body);
        Ok(LiveBody {
            html: shell(
                "ledger-journal-page",
                PageVariant::Detail,
                &self.flash,
                page,
            )?,
            ..LiveBody::default()
        })
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        if Pages::company(scope).is_none() {
            return self.simple("Manual journal", no_company("ledger-journal-no-company")?);
        }
        let journal: view::Journal = match self
            .pages
            .read(
                scope,
                "finance.ledger.journal.get",
                json!({"journal_id": self.id}),
            )
            .await?
        {
            Read::Found(value) => match view::parse(value) {
                Some(journal) => journal,
                None => {
                    return self.simple(
                        "Manual journal",
                        unavailable("ledger-journal-refused", "this journal", &Read::Failed)?,
                    )
                }
            },
            Read::Failed | Read::Refused(_) => {
                return self.simple(
                    "Manual journal",
                    render(
                        EmptyState::missing(
                            "This journal does not exist",
                            "It may belong to another company. Go back to the manual journals.",
                        )
                        .id("ledger-journal-missing"),
                    )?,
                )
            }
            other => {
                return self.simple(
                    "Manual journal",
                    unavailable("ledger-journal-refused", "this journal", &other)?,
                )
            }
        };
        let state = self.pages.book(scope).await?.ok();
        let units = state
            .as_ref()
            .and_then(|s| s.book.as_ref())
            .map_or(2, |b| b.minor_units);
        let required = state
            .as_ref()
            .and_then(|s| s.duty_separation.as_ref())
            .and_then(|c| c.required)
            .unwrap_or(true);
        let chart = self.pages.chart(scope).await?.unwrap_or_default();
        let display = self.pages.display(scope).await?;
        let can_decide = self.pages.allowed(scope, APPROVE_JOURNALS).await?;
        let waiting = journal.status == "submitted";
        let mine = scope.user() == journal.submitted_by_user_id;

        let mut content = Html::EMPTY;
        if waiting && mine && required {
            content.push(&render(Alert {
                id: "ledger-journal-own",
                kind: StatusKind::Info,
                class: "mb-3",
                body: Html::text(
                    "You submitted this journal. Your company asks someone else to approve it.",
                ),
            })?);
        }
        let total: Decimal = journal.lines.iter().map(|l| view::decimal(&l.debit)).sum();
        let mut facts = vec![
            Fact {
                title: "Number",
                id: "ledger-journal-number",
                value: Html::text(&journal.journal_number.to_string()),
            },
            Fact {
                title: "Date",
                id: "ledger-journal-date",
                value: Html::text(&journal.entry_date),
            },
            Fact {
                title: "Description",
                id: "ledger-journal-description",
                value: Html::text(&journal.description),
            },
            Fact {
                title: "Reference",
                id: "ledger-journal-reference",
                value: Html::text(journal.reference.as_deref().unwrap_or("—")),
            },
            Fact {
                title: "Currency",
                id: "ledger-journal-currency",
                value: Html::text(&journal.currency_code),
            },
            Fact {
                title: "Total",
                id: "ledger-journal-total",
                value: Html::text(&view::amount(&total.to_string(), units)),
            },
            Fact {
                title: "Status",
                id: "ledger-journal-status",
                value: status_badge(&journal.status)?,
            },
            Fact {
                title: "Submitted by",
                id: "ledger-journal-by",
                value: Html::text(&who(
                    journal.submitted_by_user_id,
                    journal.submitted_by_impersonator_id,
                )),
            },
            Fact {
                title: "Submitted",
                id: "ledger-journal-at",
                value: instant(
                    &display,
                    &journal.submitted_at,
                    "ledger-journal-submitted-at",
                )?,
            },
        ];
        if let Some(decision) = &journal.decision {
            facts.push(Fact {
                title: if decision.event == "approved" {
                    "Approved by"
                } else {
                    "Rejected by"
                },
                id: "ledger-journal-decided-by",
                value: Html::text(&who(decision.actor_user_id, decision.actor_impersonator_id)),
            });
            facts.push(Fact {
                title: if decision.event == "approved" {
                    "Approved"
                } else {
                    "Rejected"
                },
                id: "ledger-journal-decided-at",
                value: instant(
                    &display,
                    &decision.occurred_at,
                    "ledger-journal-decided-time",
                )?,
            });
            if let Some(reason) = &decision.reason {
                facts.push(Fact {
                    title: "Reason",
                    id: "ledger-journal-reason",
                    value: Html::text(reason),
                });
            }
            if let Some(entry) = decision.entry_id {
                facts.push(Fact {
                    title: "Posted as",
                    id: "ledger-journal-entry",
                    value: render(EntryLink {
                        id: entry,
                        label: "See the journal entry",
                    })?,
                });
            }
        }
        let mut head = render(SectionHeading {
            id: "ledger-journal-heading",
            title: "Journal",
            ..SectionHeading::default()
        })?;
        head.push(&render(Facts {
            id: "ledger-journal-facts",
            items: facts,
        })?);
        content.push(&render(Card {
            id: "ledger-journal-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-journal-heading",
            body: head,
            ..Card::default()
        })?);

        if self.lines.is_none() {
            self.lines = Some(
                ListColumns::mount(
                    "ledger-journal-lines",
                    vec![
                        BuiltinColumn::new("line", "Line"),
                        BuiltinColumn::new("account", "Account"),
                        BuiltinColumn::new("description", "Description"),
                        BuiltinColumn::new("debit", "Debit").right(),
                        BuiltinColumn::new("credit", "Credit").right(),
                    ],
                )
                .map_err(|_| Error::Internal)?,
            );
        }
        let columns = self.lines.as_mut().ok_or(Error::Internal)?;
        columns.load(&journal.lines, |l| l.line_number.to_string());
        let mut rows = Vec::new();
        for line in &journal.lines {
            let account = chart
                .iter()
                .find(|a| a.id == line.account_id)
                .map_or(format!("Account #{}", line.account_id), |a| {
                    format!("{} {}", a.code, a.name)
                });
            rows.push(
                FlexRow::new(line.line_number.to_string())
                    .cell("line", Cell::text(line.line_number.to_string()))
                    .slot(
                        "account",
                        render(super::AccountLink {
                            id: line.account_id,
                            label: &account,
                        })?,
                    )
                    .cell(
                        "description",
                        Cell::text(line.description.as_deref().unwrap_or("—")),
                    )
                    .slot("debit", figure(&view::amount_or_dash(&line.debit, units))?)
                    .slot(
                        "credit",
                        figure(&view::amount_or_dash(&line.credit, units))?,
                    ),
            );
        }
        let credit_total: Decimal = journal.lines.iter().map(|l| view::decimal(&l.credit)).sum();
        rows.push(
            FlexRow::new("total")
                .cell("line", Cell::text("Total"))
                .slot("debit", figure(&view::amount(&total.to_string(), units))?)
                .slot(
                    "credit",
                    figure(&view::amount(&credit_total.to_string(), units))?,
                ),
        );
        let mut table = FlexTable::new("ledger-journal-lines", columns.flex_columns(), rows);
        table.caption = "Lines of the journal";
        table.framed = false;
        table.zoom = columns.zoom;
        table.empty = Some(TableEmpty::Content(Html::text(
            "This journal has no lines.",
        )));
        let mut lines = render(SectionHeading {
            id: "ledger-journal-lines-heading",
            title: "Lines",
            class: "p-5 pb-0",
            ..SectionHeading::default()
        })?;
        lines.push(&render(table)?);
        content.push(&render(Card {
            id: "ledger-journal-lines-card",
            inner_class: "p-0",
            class: "mt-4",
            role: "region",
            labelledby: "ledger-journal-lines-heading",
            body: lines,
            ..Card::default()
        })?);

        let mut actions = Html::EMPTY;
        if waiting && can_decide {
            actions.push(&render(Button {
                id: "ledger-journal-approve",
                button_type: "button",
                event: "ask_approve",
                variant: ButtonVariant::Primary,
                body: Html::text("Approve"),
                ..Button::default()
            })?);
            actions.push(&render(Button {
                id: "ledger-journal-reject",
                button_type: "button",
                event: "open_reject",
                body: Html::text("Reject"),
                ..Button::default()
            })?);
        }
        actions.push(&render(BackLink {
            href: INDEX,
            id: "ledger-journal-back",
            title: "Back to the manual journals",
            ..BackLink::default()
        })?);
        let mut page = render(Header {
            title: Html::text(&format!("Journal {}", journal.journal_number)),
            subtitle: &journal.description,
            actions,
            ..Header::default()
        })?;
        page.push(&render(Announce {
            kind: crate::MODULE,
            record_id: &format!("journal-{}", journal.id),
        })?);
        page.push(&content);
        if let Some(review) = &self.approving {
            let exception = review.facts["approval_exception"].as_bool() == Some(true);
            page.push(&render(ConfirmDialog {
                id: "ledger-journal-confirm",
                consequence: &format!("Approve journal {} and post it?", journal.journal_number),
                detail: if exception {
                    "It posts as an entry that cannot be edited. Your company does not separate duties, so the ledger also records an approval exception for an independent reviewer."
                } else {
                    "It posts as an entry that cannot be edited. To undo it later, reverse the entry."
                },
                confirm: "Approve and post",
                working: "Posting…",
                confirm_event: "approve",
                cancel_event: "cancel_approve",
            })?);
        }
        if let Some(rejecting) = &self.rejecting {
            page.push(&render(Modal {
                id: "ledger-journal-reject-modal",
                title: "Reject journal",
                cancel_event: "close_reject",
                description: "Nothing posts. The journal stays on record with your reason.",
                body: self.reject_form(rejecting)?,
                ..Modal::default()
            })?);
        }
        Ok(LiveBody {
            html: shell(
                "ledger-journal-page",
                PageVariant::Detail,
                &self.flash,
                page,
            )?,
            rows: journal.lines.len(),
            ..LiveBody::default()
        })
    }

    fn reject_form(&self, rejecting: &Rejecting) -> Result<Html> {
        let errors: Vec<String> = rejecting
            .refusal
            .field("reason")
            .iter()
            .map(|m| error_text("reason", m))
            .collect();
        let mut body = Html::EMPTY;
        let mut messages = rejecting.refusal.alert(&["reason"]);
        if !rejecting.notice.is_empty() {
            messages.push(rejecting.notice.clone());
        }
        if !messages.is_empty() {
            body.push(&render(Alert {
                id: "ledger-journal-reject-notice",
                kind: StatusKind::Error,
                class: "mb-3",
                body: Html::text(&messages.join(" ")),
            })?);
        }
        body.push(&render(
            Field::in_form("reject", "reason")
                .id("ledger-reject-reason")
                .label("Reason")
                .kind(InputKind::Textarea)
                .value(&rejecting.reason)
                .required()
                .hint("Up to 500 characters.")
                .errors(errors)
                .attr("data-debounce", "300"),
        )?);
        let mut actions = render(Button {
            id: "ledger-reject-cancel",
            button_type: "button",
            event: "close_reject",
            body: Html::text("Cancel"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "ledger-reject-submit",
            button_type: "button",
            event: "reject",
            variant: ButtonVariant::Danger,
            working: "Rejecting…",
            body: Html::text("Reject journal"),
            ..Button::default()
        })?);
        render(Form {
            id: "ledger-reject-form",
            event: "validate",
            change: true,
            class: "mt-4",
            body,
            actions: dialog_actions(actions)?,
        })
    }

    async fn ask_approve(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        match self
            .pages
            .review(
                r.scope,
                r.credential,
                "ask_approve",
                "finance.ledger.journal.approve",
                json!({"journal_id": self.id}),
            )
            .await?
        {
            Ok(review) => self.approving = Some(review),
            Err(failure) => self.flash.error(failure.text("The journal")),
        }
        Ok(())
    }

    async fn approve(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(review) = self.approving.take() else {
            return Ok(());
        };
        match self
            .pages
            .commit(
                r.scope,
                r.credential,
                "approve",
                "finance.ledger.journal.approve",
                review,
            )
            .await?
        {
            Ok(_) => self.flash.success("The journal is approved and posted."),
            Err(failure) => self.flash.error(failure.text("The journal")),
        }
        Ok(())
    }

    async fn reject(&mut self, r: &LiveRequest<'_, Scope>) -> Result<()> {
        let Some(rejecting) = &mut self.rejecting else {
            return Ok(());
        };
        match self
            .pages
            .submit(
                r.scope,
                r.credential,
                "reject",
                "finance.ledger.journal.reject",
                json!({"journal_id": self.id, "reason": rejecting.reason.trim()}),
            )
            .await?
        {
            Ok(_) => {
                self.rejecting = None;
                self.flash
                    .success("The journal is rejected. Nothing was posted.");
            }
            Err(Failure::Refused(refusal)) => {
                rejecting.refusal = refusal;
                rejecting.notice.clear();
            }
            Err(failure) => rejecting.notice = failure.text("The rejection"),
        }
        Ok(())
    }
}

impl LiveState<Scope, Error> for Journal {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(value) = r.input {
                match event_type(value)? {
                    "ask_approve" => self.ask_approve(&r).await?,
                    "cancel_approve" => self.approving = None,
                    "approve" => self.approve(&r).await?,
                    "open_reject" => self.rejecting = Some(Rejecting::default()),
                    "close_reject" => self.rejecting = None,
                    "validate" => {
                        if let Some(rejecting) = &mut self.rejecting {
                            rejecting.reason = params::form_values(value, "reject")
                                .get("reason")
                                .cloned()
                                .unwrap_or_default();
                            rejecting.refusal = Refusal::default();
                            rejecting.notice.clear();
                        }
                    }
                    "reject" => self.reject(&r).await?,
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                }
            }
            self.view(r.scope).await
        })
    }
}
