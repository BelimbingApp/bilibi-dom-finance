//! Mirrors Bilimbi `Finance.Reconciliation.Web.ReconciliationLive`: the
//! statement balance, the book balance, the items that explain the
//! difference, and the difference itself. An open reconciliation reads its
//! figures now, follows the ledger's postings, lets a statement line the books
//! lack be posted as a bank charge or interest, and is finalised once the
//! difference is zero or abandoned with a reason; a finalised one shows the
//! figures and items it was finalised with, which never change. Every write is
//! a facade call that re-asks its capability.
use super::{clear_flash, company, event_type, flash_map, format::told, render, Follow, Pages};
use crate::{
    BankAccount, BankReconciliation, ItemKind, PostLine, ReconciliationStatus,
    FINALISE_RECONCILIATIONS, MANAGE_RECONCILIATIONS, POST_STATEMENT_LINES,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_datetime::{Format, Precision};
use bilibi_finance_ledger::{MinorUnits, POST_ENTRIES};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::*, params, Badge, BadgeKind, BuiltinColumn, Cell, EmptyState, Fact, Facts, Field,
    FlashMap, FlexRow, FlexTable, Html, Instant, ListColumns, RowMode, StatusKind, TableEmpty,
};

pub(super) struct Detail {
    pages: Pages,
    id: i64,
    report: Option<BankReconciliation>,
    account: Option<BankAccount>,
    refusal: Option<String>,
    can_finalise: bool,
    can_abandon: bool,
    can_post: bool,
    abandoning: bool,
    reason: String,
    items: ListColumns,
    events: ListColumns,
    flash: Option<(StatusKind, String)>,
    follow: Follow,
}

#[derive(Template)]
#[template(path = "reconciliation.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    back: String,
    back_title: String,
    refusal: Html,
    record: Option<RecordView>,
}
struct RecordView {
    id: i64,
    title: String,
    title_actions: Html,
    subtitle: Html,
    actions: Html,
    figures: Html,
    controls: Html,
    items: Option<(Option<usize>, Html)>,
    events: Html,
    abandon: Option<Field>,
}

#[derive(Template)]
#[template(
    source = r#"<span id="reconciliation-status">{{ status|safe }}</span>{% if let Some(through) = through %}<span id="reconciliation-through" class="ml-2">{{ through }}</span>{% endif %}"#,
    ext = "html"
)]
struct Subtitle {
    status: Html,
    through: Option<String>,
}

#[derive(Template)]
#[template(
    source = r#"<a id="reconciliation-matching" href="{{ href }}">Match statement lines</a>"#,
    ext = "html"
)]
struct Actions {
    href: String,
}

#[derive(Template)]
#[template(
    source = r#"<span class="tabular-nums">{{ text }}</span>{% if let Some(badge) = badge %} {{ badge|safe }}{% endif %}"#,
    ext = "html"
)]
struct Figure {
    text: String,
    badge: Option<Html>,
}

#[derive(Template)]
#[template(
    source = r#"{% import "components.html" as ui %}{% if open %}<div class="mt-4 flex flex-wrap gap-2">{% if finalise %}{% call ui::button(id="reconciliation-finalise", button_type="button", variant=bilibi_ui::ButtonVariant::Primary, event="finalise", disabled=unexplained, working="Finalising…") %}Finalise{% endcall %}{% endif %}{% if abandon %}{% call ui::button(id="reconciliation-abandon", button_type="button", event="abandoning") %}Abandon{% endcall %}{% endif %}</div>{% endif %}{% if let Some(forbidden) = forbidden %}<div class="mt-4">{{ forbidden|safe }}</div>{% endif %}"#,
    ext = "html"
)]
struct Controls {
    open: bool,
    finalise: bool,
    abandon: bool,
    unexplained: bool,
    forbidden: Option<Html>,
}

#[derive(Template)]
#[template(
    source = r#"{% import "components.html" as ui %}<span class="whitespace-nowrap">{% if let Some(line) = charge %}{% call ui::button(id="item-{}-post-charge"|format(line), button_type="button", event="post_charge", name="line", value=line.to_string(), working="Posting…") %}Post as bank charge{% endcall %}{% endif %}{% if let Some(line) = interest %}{% call ui::button(id="item-{}-post-interest"|format(line), button_type="button", event="post_interest", name="line", value=line.to_string(), working="Posting…") %}Post as interest{% endcall %}{% endif %}</span>"#,
    ext = "html"
)]
struct Post {
    charge: Option<i64>,
    interest: Option<i64>,
}

#[derive(Template)]
#[template(
    source = r#"{{ text }}{% if let Some(reference) = reference %}<span class="block text-xs text-ink-muted">{{ reference }}</span>{% endif %}"#,
    ext = "html"
)]
struct Described<'a> {
    text: &'a str,
    reference: Option<&'a str>,
}

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::StatementLine => "On the statement, not in the books",
        ItemKind::Entry => "In the books, not on the statement",
        ItemKind::Timing => "Matched, cleared after the statement date",
        ItemKind::OpeningItem => "Opening item not yet cleared",
    }
}

fn event_label(action: &str) -> &str {
    match action {
        "opened" => "Opened",
        "finalised" => "Finalised",
        "abandoned" => "Abandoned",
        other => other,
    }
}

fn badge(kind: BadgeKind, text: &str) -> Result<Html> {
    render(Badge {
        kind,
        dot: false,
        class: "",
        body: Html::text(text),
    })
}

fn columns(id: &str, values: &[(&str, &str)]) -> Result<ListColumns> {
    ListColumns::mount(
        id,
        values
            .iter()
            .map(|(key, label)| {
                let column = BuiltinColumn::new(key, label);
                if *key == "amount" {
                    column.right()
                } else {
                    column
                }
            })
            .collect(),
    )
    .map_err(|_| Error::Internal)
}

impl Detail {
    pub(super) fn new(pages: &Pages, id: i64) -> Result<Self> {
        Ok(Self {
            pages: pages.clone(),
            id,
            report: None,
            account: None,
            refusal: None,
            can_finalise: false,
            can_abandon: false,
            can_post: false,
            abandoning: false,
            reason: String::new(),
            items: columns(
                "reconciliation-items-table",
                &[
                    ("kind", "Kind"),
                    ("date", "Date"),
                    ("description", "Description"),
                    ("amount", "Amount"),
                ],
            )?,
            events: columns(
                "reconciliation-events-table",
                &[
                    ("event", "Event"),
                    ("by", "By"),
                    ("when", "When"),
                    ("reason", "Reason"),
                ],
            )?,
            flash: None,
            follow: Follow::default(),
        })
    }
    pub(super) fn follow(mut self, follow: Follow) -> Self {
        self.follow = follow;
        self
    }

    /// Bilimbi `load`: the reconciliation and its bank account, or the
    /// refusal the page shows in their place.
    async fn load(&mut self, scope: &Scope) -> Result<()> {
        let company = company(scope)?;
        let r = &self.pages.reconciliation;
        let read = match told(r.get_reconciliation(scope, company, self.id).await)? {
            Ok(report) => match told(
                r.get_bank_account(scope, company, report.bank_account_id)
                    .await,
            )? {
                Ok(account) => Ok((report, account)),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        match read {
            Ok((report, account)) => {
                self.report = Some(report);
                self.account = Some(account);
                self.refusal = None;
                self.can_finalise = self.pages.allowed(scope, FINALISE_RECONCILIATIONS).await?;
                self.can_abandon = self.pages.allowed(scope, MANAGE_RECONCILIATIONS).await?;
                self.can_post = self.pages.allowed(scope, POST_STATEMENT_LINES).await?
                    && self.pages.allowed(scope, POST_ENTRIES).await?;
            }
            Err(e) => {
                self.report = None;
                self.account = None;
                self.refusal = Some(e);
            }
        }
        Ok(())
    }

    /// Bilimbi `authorize_event`: refused, the page says so and nothing else
    /// happens.
    async fn may(&mut self, scope: &Scope, capabilities: &[&str]) -> Result<bool> {
        for capability in capabilities {
            if !self.pages.allowed(scope, capability).await? {
                self.flash = Some((StatusKind::Error, super::DENIED.into()));
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Handles one event; whether the reconciliation is to be read again.
    async fn event(&mut self, scope: &Scope, v: &serde_json::Value) -> Result<bool> {
        let r = self.pages.reconciliation.clone();
        match event_type(v)? {
            "clear-flash" => {
                clear_flash(&mut self.flash, v);
                Ok(false)
            }
            grid @ ("items_grid" | "events_grid") => {
                let columns = if grid == "items_grid" {
                    &mut self.items
                } else {
                    &mut self.events
                };
                columns.handle(&params::string_values(v));
                Ok(false)
            }
            "finalise" => {
                if !self.may(scope, &[FINALISE_RECONCILIATIONS]).await? {
                    return Ok(false);
                }
                let result = r
                    .finalise_reconciliation(scope, company(scope)?, self.id)
                    .await;
                self.flash = Some(match told(result)? {
                    Ok(done) => (
                        StatusKind::Success,
                        format!(
                            "Reconciliation {} finalised; its figures are now fixed.",
                            done.reconciliation_number
                        ),
                    ),
                    Err(e) => (StatusKind::Error, e),
                });
                Ok(true)
            }
            kind @ ("post_charge" | "post_interest") => {
                if !self
                    .may(scope, &[POST_STATEMENT_LINES, POST_ENTRIES])
                    .await?
                {
                    return Ok(false);
                }
                let line = params::positive_id(v["line"].as_str()).unwrap_or(0);
                let post = PostLine {
                    kind: if kind == "post_charge" {
                        "charge"
                    } else {
                        "interest"
                    }
                    .into(),
                    description: None,
                };
                let result = r
                    .post_statement_line(scope, company(scope)?, line, &post)
                    .await;
                self.flash = Some(match told(result)? {
                    Ok(posted) => (
                        StatusKind::Success,
                        format!("Posted as entry {} and matched.", posted.entry.entry_number),
                    ),
                    Err(e) => (StatusKind::Error, e),
                });
                Ok(true)
            }
            "abandoning" => {
                self.abandoning = true;
                self.reason.clear();
                Ok(false)
            }
            "cancel_abandon" => {
                self.abandoning = false;
                Ok(false)
            }
            "abandon_change" => {
                self.reason = v["abandon[reason]"].as_str().unwrap_or_default().into();
                Ok(false)
            }
            "abandon" => {
                self.reason = v["abandon[reason]"].as_str().unwrap_or_default().into();
                if !self.may(scope, &[MANAGE_RECONCILIATIONS]).await? {
                    return Ok(false);
                }
                let result = r
                    .abandon_reconciliation(scope, company(scope)?, self.id, Some(&self.reason))
                    .await;
                match told(result)? {
                    Ok(_) => {
                        self.flash =
                            Some((StatusKind::Info, "The reconciliation was abandoned.".into()));
                        self.abandoning = false;
                        Ok(true)
                    }
                    Err(e) => {
                        self.flash = Some((StatusKind::Error, e));
                        Ok(false)
                    }
                }
            }
            _ => Err(Error::Invalid),
        }
    }

    async fn body(&self, scope: &Scope) -> Result<String> {
        let mut view = View {
            flash: flash_map(&self.flash),
            back: self.account.as_ref().map_or_else(
                || "/finance/reconciliation".into(),
                |a| format!("/finance/reconciliation/accounts/{}", a.id),
            ),
            back_title: self
                .account
                .as_ref()
                .map_or_else(|| "Bank accounts".into(), |a| a.name.clone()),
            refusal: match &self.refusal {
                Some(reason) => render(
                    EmptyState::missing("Reconciliation unavailable", reason)
                        .id("reconciliation-refusal"),
                )?,
                None => Html::EMPTY,
            },
            record: None,
        };
        let (Some(report), Some(account)) = (&self.report, &self.account) else {
            return view.render().map_err(|_| Error::Internal);
        };
        let open = report.status == ReconciliationStatus::Open;
        let figures = report.figures.as_ref();
        let units = figures.map_or(2, |f| f.minor_units.get());

        let status = badge(
            match report.status {
                ReconciliationStatus::Open => BadgeKind::Warning,
                ReconciliationStatus::Finalised => BadgeKind::Success,
                ReconciliationStatus::Abandoned => BadgeKind::Neutral,
            },
            report.status.as_str(),
        )?;
        let subtitle = render(Subtitle {
            status,
            through: figures.map(|f| {
                format!(
                    "through entry {}, statement {}, match event {}",
                    f.through_entry_number,
                    f.through_statement_number,
                    f.through_match_event_number
                )
            }),
        })?;

        let mut facts = vec![Fact {
            title: "Balance per statement",
            id: "figure-statement-balance",
            value: render(Figure {
                text: format!(
                    "{} {}",
                    report
                        .statement_balance
                        .display(MinorUnits::new(i64::from(units))),
                    figures.map_or("", |f| f.currency_code.as_str())
                ),
                badge: None,
            })?,
        }];
        if let Some(f) = figures {
            facts.push(Fact {
                title: "Balance per books",
                id: "figure-book-balance",
                value: render(Figure {
                    text: f.book_balance.display(MinorUnits::new(i64::from(units))),
                    badge: None,
                })?,
            });
            let explained = f.difference.is_zero();
            facts.push(Fact {
                title: "Unexplained difference",
                id: "figure-difference",
                value: render(Figure {
                    text: f.difference.display(MinorUnits::new(i64::from(units))),
                    badge: Some(if explained {
                        badge(BadgeKind::Success, "explained")?
                    } else {
                        badge(BadgeKind::Warning, "not yet explained")?
                    }),
                })?,
            });
        }
        let figures_html = render(Facts {
            id: "reconciliation-facts",
            items: facts,
        })?;
        let controls = render(Controls {
            open,
            finalise: self.can_finalise,
            abandon: self.can_abandon,
            unexplained: figures.is_none_or(|f| !f.difference.is_zero()),
            forbidden: if open && !self.can_finalise {
                Some(render(
                    EmptyState::forbidden("finalise reconciliations")
                        .id("reconciliation-finalise-forbidden"),
                )?)
            } else {
                None
            },
        })?;

        let items = match figures {
            Some(f) => {
                let mode = self.items.mode();
                let mut rows = vec![];
                for (index, item) in f.items.iter().enumerate() {
                    let date = item.item_date.to_string();
                    let mut row = FlexRow::new((index + 1).to_string())
                        .cell("kind", Cell::text(kind_label(item.kind)))
                        .cell("date", Cell::date(&date, &date))
                        .cell("description", Cell::text(&item.description))
                        .slot(
                            "description",
                            render(Described {
                                text: &item.description,
                                reference: item
                                    .reference
                                    .as_deref()
                                    .filter(|_| mode != RowMode::Compact),
                            })?,
                        )
                        .cell(
                            "amount",
                            Cell::text(item.amount.display(MinorUnits::new(i64::from(units)))),
                        );
                    row.dom_id = Some(format!("item-{}", index + 1));
                    if open && self.can_post && item.kind == ItemKind::StatementLine {
                        let line = item.statement_line_id;
                        let negative =
                            item.amount.amount().is_sign_negative() && !item.amount.is_zero();
                        row.actions = render(Post {
                            charge: line.filter(|_| negative),
                            interest: line.filter(|_| item.amount.is_positive()),
                        })?;
                    }
                    rows.push(row);
                }
                let mut table = FlexTable::new(&self.items.id, self.items.flex_columns(), rows);
                table.event = "items_grid";
                table.framed = false;
                table.actions = true;
                table.zoom = self.items.zoom;
                table.suggestions = self.items.flex_suggestions();
                table.add_query = &self.items.add_query;
                table.empty = Some(TableEmpty::State(EmptyState::missing(
                    "Nothing to explain",
                    "Every statement line through the end date is matched, and so is every entry.",
                )));
                Some((Some(f.items.len()), render(table)?))
            }
            None => None,
        };

        let display = self.pages.display(scope).await?;
        let mut rows = vec![];
        for e in &report.events {
            let time = display.render(e.recorded_at, Format::DateTime, Precision::Second);
            let mut instant = Instant::new(
                &time.iso,
                &time.text,
                &time.company,
                &time.utc,
                time.mode.as_str(),
            );
            instant.precision = "second";
            let by = match e.recorded_by_impersonator_id {
                Some(i) => format!("user {} (impersonated by {i})", e.recorded_by_user_id),
                None => format!("user {}", e.recorded_by_user_id),
            };
            let mut row = FlexRow::new(e.event_number.to_string())
                .cell("event", Cell::text(event_label(&e.action)))
                .cell("by", Cell::text(by))
                .cell("when", Cell::instant(instant))
                .cell("reason", Cell::text(e.reason.clone().unwrap_or_default()));
            row.dom_id = Some(format!("event-{}", e.event_number));
            rows.push(row);
        }
        let mut table = FlexTable::new(&self.events.id, self.events.flex_columns(), rows);
        table.event = "events_grid";
        table.framed = false;
        table.zoom = self.events.zoom;
        table.suggestions = self.events.flex_suggestions();
        table.add_query = &self.events.add_query;
        let events = render(table)?;

        let title = format!(
            "Reconciliation {} as of {}",
            report.reconciliation_number, report.end_date
        );
        view.record = Some(RecordView {
            id: report.id,
            title_actions: bilibi_ui::Html::EMPTY,
            title,
            subtitle,
            actions: render(Actions {
                href: format!("/finance/reconciliation/accounts/{}/matching", account.id),
            })?,
            figures: figures_html,
            controls,
            items,
            events,
            abandon: self.abandoning.then(|| {
                Field::in_form("abandon", "reason")
                    .id("abandon-reason")
                    .label("Reason")
                    .value(&self.reason)
                    .attr("maxlength", "500")
                    .required()
            }),
        });
        view.render().map_err(|_| Error::Internal)
    }
}

impl LiveState<Scope, Error> for Detail {
    fn subscriptions(&mut self) -> Vec<Subscription> {
        self.follow.subscriptions()
    }
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let reload = match request.input {
                // The mount's first read, or a posting notice: an open
                // reconciliation follows the books; a finalised one cannot
                // change.
                LiveInput::Refresh => {
                    let posted = self.follow.posted();
                    !posted
                        || self
                            .report
                            .as_ref()
                            .is_none_or(|r| r.status == ReconciliationStatus::Open)
                }
                LiveInput::Params(_) => true,
                LiveInput::Event(v) => self.event(request.scope, v).await?,
            };
            if reload {
                self.load(request.scope).await?;
            }
            Ok(LiveBody {
                html: self.body(request.scope).await?,
                ..Default::default()
            })
        })
    }
}
