//! One account group: read first, edit in place. Its name and the group it
//! sits in change on the spot. Below the facts, what it holds: its accounts
//! with their own balances and the groups inside it with their totals. The
//! group's total is the sum of every account under it; it takes no postings
//! and has no balance of its own.
use super::{
    account_name, event_type, figure,
    groups::{holds, INDEX},
    no_company, render, shell, unavailable, view, Failure, Flash, Pages,
};
use crate::MANAGE_ACCOUNTS;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    BackLink, BuiltinColumn, Card, Cell, Choice, Combobox, CommitStates, CommitStatus, Fact, Facts,
    FlexRow, FlexTable, Header, Html, InlineChoice, InlineEdit, InlineField, ListColumns,
    PageVariant, SectionHeading, TableEmpty,
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) struct Group {
    pages: Pages,
    id: i64,
    company: Option<i64>,
    flash: Flash,
    statuses: CommitStates,
    columns: Option<ListColumns>,
}

impl Group {
    pub(super) fn new(pages: Pages, id: i64, company: Option<i64>) -> Self {
        Self {
            pages,
            id,
            company,
            flash: Flash::default(),
            statuses: CommitStates::default(),
            columns: None,
        }
    }

    fn inline(&self, key: &str, label: &str, value: &str, editable: bool) -> InlineField {
        let mut field = InlineField::new(format!("ledger-group-{key}"), key, label, value)
            .version("1")
            .status(self.statuses.get(key));
        field.id_value = self.id.to_string();
        field.save_event = "save_group_field".into();
        // "No group" is a real choice.
        if key == "parent" {
            field = field.allow_empty();
        }
        if !editable {
            field = field.readonly();
        }
        field
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        if self.company.is_none() {
            return self.page("Account group", no_company("ledger-group-no-company")?);
        }
        let company = self.company.ok_or(Error::Invalid)?;
        let chart = match self.pages.chart_in(scope, company).await? {
            Ok(chart) => chart,
            Err(refused) => {
                return self.page(
                    "Account group",
                    unavailable("ledger-group-refused", "this account group", &refused)?,
                );
            }
        };
        let Some(group) = chart
            .iter()
            .find(|a| a.id == self.id && a.is_group)
            .cloned()
        else {
            return self.page("Account group", super::groups::missing()?);
        };
        let editable = self.pages.allowed(scope, MANAGE_ACCOUNTS).await?;
        let figures = self.pages.chart_figures_in(scope, company).await?;
        let units = figures.as_ref().map(|f| f.minor_units);
        let figure_of = |id: i64| {
            figures
                .as_ref()
                .and_then(|f| f.rows.iter().find(|r| r.id == id))
        };
        let amount =
            |text: Option<&String>| -> Option<String> { Some(view::amount(text?, units?)) };

        // A group sits in another group of its type, never in itself or in
        // what it holds.
        let mut inside: HashSet<i64> = HashSet::from([group.id]);
        loop {
            let before = inside.len();
            for a in chart.iter().filter(|a| a.is_group) {
                if a.parent_id.is_some_and(|p| inside.contains(&p)) {
                    inside.insert(a.id);
                }
            }
            if inside.len() == before {
                break;
            }
        }
        let others = view::by_name(chart.iter().filter(|a| {
            a.is_group && a.account_type == group.account_type && !inside.contains(&a.id)
        }));
        let parent_display = match group
            .parent_id
            .and_then(|id| chart.iter().find(|a| a.id == id))
        {
            Some(parent) => account_name(parent)?,
            None => Html::text("None"),
        };
        let mut options = vec![Choice::new("No group", "")];
        options.extend(others.iter().map(|a| view::account_choice(a)));
        let editor = render(
            Combobox::new("ledger-group-parent-input", "parent")
                .aria_label("Group")
                .placeholder("No group")
                .wrapper_class("mb-0")
                .value(group.parent_id.map(|p| p.to_string()).unwrap_or_default())
                .options(options),
        )?;
        let members: Vec<&view::Account> =
            view::by_name(chart.iter().filter(|a| a.parent_id == Some(group.id)));
        let held = members.iter().fold((0, 0), |(accounts, groups), a| {
            if a.is_group {
                (accounts, groups + 1)
            } else {
                (accounts + 1, groups)
            }
        });
        let total = amount(figure_of(group.id).and_then(|r| r.total.as_ref()));
        let items = vec![
            Fact {
                title: "Code",
                id: "ledger-group-code",
                value: Html::text(&group.code),
            },
            Fact {
                title: "Name",
                id: "ledger-group-name",
                value: render(InlineEdit {
                    field: self.inline("name", "Name", &group.name, editable),
                })?,
            },
            Fact {
                title: "Type",
                id: "ledger-group-type",
                value: Html::text(view::type_label(&group.account_type)),
            },
            Fact {
                title: "Group",
                id: "ledger-group-group",
                value: render(InlineChoice {
                    field: self.inline(
                        "parent",
                        "Group",
                        &group.parent_id.map(|p| p.to_string()).unwrap_or_default(),
                        editable,
                    ),
                    display: parent_display,
                    editor,
                })?,
            },
            Fact {
                title: "Holds",
                id: "ledger-group-holds",
                value: Html::text(&holds(held.0, held.1)),
            },
            Fact {
                title: "Total today",
                id: "ledger-group-total",
                value: Html::text(total.as_deref().unwrap_or("—")),
            },
        ];
        let mut card = render(SectionHeading {
            id: "ledger-group-heading",
            title: "Account group",
            ..SectionHeading::default()
        })?;
        card.push(&render(Facts {
            id: "ledger-group-facts",
            items,
        })?);
        let mut content = render(Card {
            id: "ledger-group-card",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-group-heading",
            body: card,
            ..Card::default()
        })?;

        let columns = self.columns.get_or_insert_with(|| {
            ListColumns::mount(
                "ledger-group-members-table",
                vec![
                    BuiltinColumn::new("code", "Code"),
                    BuiltinColumn::new("name", "Name"),
                    BuiltinColumn::new("kind", "Kind"),
                    BuiltinColumn::new("figure", "Balance or total"),
                ],
            )
            .expect("group members columns")
        });
        columns.load(&members, |a| a.id.to_string());
        let mut rows = Vec::new();
        for member in &members {
            let row = figure_of(member.id);
            let shown = amount(row.and_then(|r| r.balance.as_ref().or(r.total.as_ref())));
            rows.push(
                FlexRow::new(member.id.to_string())
                    .slot(
                        "code",
                        render(bilibi_ui::RecordLink {
                            kind: crate::MODULE,
                            record_id: &member.id.to_string(),
                            href: &format!(
                                "{}/{}?company={company}",
                                if member.is_group {
                                    INDEX
                                } else {
                                    super::accounts::INDEX
                                },
                                member.id
                            ),
                            workspace: false,
                            class: "text-link hover:underline",
                            body: Html::text(&member.code),
                        })?,
                    )
                    .cell("name", Cell::text(&member.name))
                    .cell(
                        "kind",
                        Cell::text(if member.is_group { "Group" } else { "Account" }),
                    )
                    .slot(
                        "figure",
                        match shown {
                            Some(text) => figure(&text)?,
                            None => Html::text("—"),
                        },
                    ),
            );
        }
        let mut table = FlexTable::new("ledger-group-members-table", columns.flex_columns(), rows);
        table.caption = "What this group holds";
        table.framed = false;
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.empty = Some(TableEmpty::Content(Html::text(
            "This group holds nothing yet. Add an account to it from the chart of accounts.",
        )));
        let mut members_card = render(SectionHeading {
            id: "ledger-group-members-heading",
            title: "Holds",
            ..SectionHeading::default()
        })?;
        members_card.push(&render(table)?);
        content.push(&render(Card {
            id: "ledger-group-members",
            inner_class: "p-5 sm:p-6",
            role: "region",
            labelledby: "ledger-group-members-heading",
            body: members_card,
            ..Card::default()
        })?);

        let mut actions = Html::EMPTY;
        actions.push(&render(BackLink {
            href: &format!("{INDEX}?company={company}"),
            id: "ledger-group-back",
            title: "Back to the account groups",
            ..BackLink::default()
        })?);
        let company_name = self.pages.company_name_in(scope, company).await?;
        let name = format!("{} · {} · {}", group.name, group.code, company_name);
        let url = format!("{INDEX}/{}?company={company}", group.id);
        let mut page = render(Header {
            title: account_name(&group)?,
            subtitle: &company_name,
            title_actions: super::record_pin("ledger-group-pin", "account group", &name, &url)?,
            actions,
        })?;
        page.push(&render(super::Announce {
            kind: crate::MODULE,
            record_id: &group.id.to_string(),
        })?);
        page.push(&content);
        Ok(LiveBody {
            html: shell("ledger-group-page", PageVariant::Detail, &self.flash, page)?,
            query: self
                .company
                .map(|id| std::collections::HashMap::from([("company".into(), id.to_string())]))
                .unwrap_or_default(),
            ..LiveBody::default()
        })
    }

    fn page(&self, title: &str, body: Html) -> Result<LiveBody> {
        let mut page = render(Header {
            title: Html::text(title),
            ..Header::default()
        })?;
        page.push(&body);
        Ok(LiveBody {
            html: shell("ledger-group-page", PageVariant::Detail, &self.flash, page)?,
            query: self
                .company
                .map(|id| std::collections::HashMap::from([("company".into(), id.to_string())]))
                .unwrap_or_default(),
            ..LiveBody::default()
        })
    }

    async fn save_field(&mut self, r: &LiveRequest<'_, Scope>, value: &Value) -> Result<()> {
        let field = value["field"].as_str().ok_or(Error::Invalid)?;
        let submitted = value["value"].as_str().ok_or(Error::Invalid)?;
        if !["name", "parent"].contains(&field)
            || value["id"].as_str() != Some(&self.id.to_string())
        {
            return Err(Error::Invalid);
        }
        let mut input = json!({"account_id": self.id});
        match field {
            "name" => input["name"] = json!(submitted.trim()),
            _ => match submitted.parse::<i64>() {
                Ok(parent) => input["parent_id"] = json!(parent),
                Err(_) => input["clear_parent"] = json!(true),
            },
        }
        let label = if field == "name" { "Name" } else { "Group" };
        let status = match self
            .pages
            .submit_in(
                r.scope,
                self.company.ok_or(Error::Invalid)?,
                r.credential,
                "save_group_field",
                "finance.ledger.account_group.update",
                input,
            )
            .await?
        {
            Ok(_) => CommitStatus::Saved,
            Err(Failure::Refused(refusal)) => {
                let mut reasons = refusal.field(if field == "name" { "name" } else { "parent_id" });
                reasons.extend(refusal.alert(&["name", "parent_id"]));
                let reasons: Vec<&str> = reasons.iter().map(String::as_str).collect();
                CommitStatus::refusal(label, submitted, &reasons)
            }
            Err(Failure::Denied) => CommitStatus::Error(super::DENIED.into()),
            Err(Failure::Changed) => CommitStatus::Error(
                "The group changed while you were editing it. Reload the page and try again."
                    .into(),
            ),
            Err(Failure::Failed) => CommitStatus::failure(),
        };
        self.statuses.put(field, status);
        Ok(())
    }
}

impl LiveState<Scope, Error> for Group {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Params(query) = r.input {
                let company = super::target_company(query, Some(r.scope))?;
                if company != self.company {
                    self.company = company;
                    self.statuses = CommitStates::default();
                }
            }
            if let LiveInput::Event(value) = r.input {
                match event_type(value)? {
                    "save_group_field" => self.save_field(&r, value).await?,
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                }
            }
            self.view(r.scope).await
        })
    }
}
