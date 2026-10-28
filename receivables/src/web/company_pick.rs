//! Choosing the Core company a customer is, shared by Add customer and Link
//! to company: an existing company found by name through Base Records and
//! the shared `RecordPicker`, or a new one created through Core's own
//! reviewed contract (`core.company.create`), whose duplicate review the
//! dialog shows before the person creates anyway with a reason. The
//! customer relationship is then recorded from the current company through
//! `core.company.relationship.assign`, Core's reviewed write.
//!
//! These are three writes, each reviewed and audited by its owner, not one
//! transaction: Company exports neither its create nor a role or
//! relationship read on a caller's connection, so Receivables cannot compose
//! them in its own (TODO(platform)). The dialog says what was done when a
//! later step is refused.
use super::{
    format::{self as f, Errors},
    render, Pages, Reviewed, Written,
};
use bilibi_database::{Error, Result};
use bilibi_records::{Hit, Reference, Resolved};
use bilibi_tenancy::Scope;
use bilibi_ui::{Button, Field, Html, InputKind, RecordPicker, StatusKind};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

const KINDS: [&str; 1] = ["company"];
/// The new company's fields, in Core's names: `(key, label, kind, maximum length, required)`.
const NEW_FIELDS: &[(&str, &str, InputKind, &str, bool)] = &[
    ("name", "Name", InputKind::Text, "200", true),
    (
        "registration_number",
        "Registration number",
        InputKind::Text,
        "100",
        false,
    ),
    ("tax_id", "Tax ID", InputKind::Text, "100", false),
    ("email", "Email", InputKind::Email, "255", false),
    ("phone", "Phone", InputKind::Tel, "50", false),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Existing,
    New,
}

/// A likely duplicate Core named for the new company.
#[derive(Clone, Debug)]
struct Candidate {
    id: i64,
    name: String,
    status: String,
    score: String,
}

/// What choosing came to.
pub(super) enum Chosen {
    /// The company to link, proven by Core.
    Company(i64),
    /// Errors to show on the dialog's company fields.
    Invalid(Errors),
    Refused(String),
    /// Likely duplicates were found; the dialog now shows them and waits
    /// for a choice or a reason.
    Review,
}

/// Whether the current company records the customer relationship.
pub(super) enum Relation {
    Recorded {
        since: Option<String>,
    },
    Missing,
    /// The reader may not read relationships.
    Hidden,
}

pub(super) struct Chooser {
    prefix: String,
    form: &'static str,
    pub mode: Mode,
    query: String,
    hits: Vec<Hit>,
    pub selected: Option<Resolved>,
    new: HashMap<String, String>,
    candidates: Vec<Candidate>,
    likely: bool,
    reason: String,
    /// The company this dialog created; a later refusal keeps it chosen so
    /// trying again links to it instead of creating it twice.
    pub created: Option<i64>,
}

impl Chooser {
    pub(super) fn new(prefix: &str, form: &'static str) -> Self {
        Self {
            prefix: prefix.into(),
            form,
            mode: Mode::Existing,
            query: String::new(),
            hits: Vec::new(),
            selected: None,
            new: HashMap::new(),
            candidates: Vec::new(),
            likely: false,
            reason: String::new(),
            created: None,
        }
    }

    /// The posted fields `(path, dom id, label)`, for the error summary.
    pub(super) fn labels(&self) -> Vec<(String, String, String)> {
        let mut labels = vec![(
            "company".into(),
            format!("{}-company-search", self.prefix),
            "Company".into(),
        )];
        for (key, label, ..) in NEW_FIELDS {
            labels.push((
                format!("company_{key}"),
                format!("{}-company-{key}", self.prefix),
                (*label).to_owned(),
            ));
        }
        labels.push((
            "company_reason".into(),
            format!("{}-company-reason", self.prefix),
            "Reason".into(),
        ));
        labels
    }

    /// Takes the dialog's posted values; answers whether the search text
    /// changed, so the caller searches again.
    pub(super) fn read(&mut self, form: &HashMap<String, String>) -> bool {
        if let Some(mode) = form.get("company_mode") {
            self.mode = if mode == "new" {
                Mode::New
            } else {
                Mode::Existing
            };
        }
        for (key, ..) in NEW_FIELDS {
            if let Some(value) = form.get(&format!("company_{key}")) {
                self.new.insert((*key).to_owned(), value.trim().to_owned());
            }
        }
        if let Some(reason) = form.get("company_reason") {
            self.reason = reason.trim().to_owned();
        }
        match form.get("company_search") {
            Some(query) if self.selected.is_none() && *query != self.query => {
                self.query = query.clone();
                true
            }
            _ => false,
        }
    }

    pub(super) async fn search(&mut self, pages: &Pages, scope: &Scope) -> Result<()> {
        self.hits = if self.query.trim().is_empty() {
            Vec::new()
        } else {
            pages.records.search(scope, &KINDS, &self.query).await?
        };
        Ok(())
    }

    /// The person chose a hit (`record=kind:id`); Base Records names it
    /// again with their permissions, so a forged value stays unnamed.
    pub(super) async fn pick(&mut self, pages: &Pages, scope: &Scope, record: &str) -> Result<()> {
        let (kind, id) = record.split_once(':').ok_or(Error::Invalid)?;
        if kind != KINDS[0] {
            return Err(Error::Invalid);
        }
        self.choose(pages, scope, id).await
    }

    async fn choose(&mut self, pages: &Pages, scope: &Scope, id: &str) -> Result<()> {
        let named = pages
            .records
            .resolve(scope, &[Reference::new(KINDS[0], id)])
            .await?;
        self.selected = named.into_iter().next();
        self.mode = Mode::Existing;
        self.hits.clear();
        self.query.clear();
        Ok(())
    }

    pub(super) fn clear(&mut self) {
        self.selected = None;
        self.hits.clear();
        self.query.clear();
    }

    /// The person took a duplicate candidate instead of creating a company.
    pub(super) async fn use_candidate(
        &mut self,
        pages: &Pages,
        scope: &Scope,
        id: &str,
    ) -> Result<()> {
        let id: i64 = id.trim().parse().map_err(|_| Error::Invalid)?;
        if !self.candidates.iter().any(|c| c.id == id) {
            return Err(Error::Invalid);
        }
        self.candidates.clear();
        self.likely = false;
        self.reason.clear();
        self.choose(pages, scope, &id.to_string()).await
    }

    /// The company section of the dialog: which company, found or new.
    pub(super) fn render(&self, errors: &Errors, framed: bool) -> Result<Html> {
        let prefix = &self.prefix;
        let mut body = render(
            Field::in_form(self.form, "company_mode")
                .id(format!("{prefix}-company-mode"))
                .label("Which company")
                .kind(InputKind::Select)
                .options(bilibi_ui::choices(&[
                    ("A company already in Companies", "existing"),
                    ("A new company", "new"),
                ]))
                .value(match self.mode {
                    Mode::Existing => "existing",
                    Mode::New => "new",
                })
                .hint("A customer is a company in Companies: its name, tax and registration numbers, contact details and billing address are read from there.")
                .wrapper_class("mb-3 sm:col-span-2"),
        )?;
        match self.mode {
            Mode::Existing => {
                let selected = self.selected.as_ref().map(|s| {
                    (
                        format!("{}:{}", KINDS[0], s.id),
                        s.pill(format!("{prefix}-company-pill"), framed),
                    )
                });
                let picker = RecordPicker::new(
                    &format!("{prefix}-company"),
                    self.form,
                    "company",
                    "Company",
                    &self.query,
                    &pages_hits(&self.hits),
                    selected
                        .as_ref()
                        .map(|(value, pill)| (value.as_str(), pill.clone())),
                    ("pick_company", "clear_company"),
                    errors.under("company", "Company"),
                );
                body.push(&render(Wrapped {
                    class: "mb-3 sm:col-span-2",
                    body: render(picker)?,
                })?);
            }
            Mode::New => {
                for (key, label, kind, max, required) in NEW_FIELDS {
                    let path = format!("company_{key}");
                    let mut field = Field::in_form(self.form, &path)
                        .id(format!("{prefix}-company-{key}"))
                        .label(*label)
                        .kind(*kind)
                        .value(self.new.get(*key).cloned().unwrap_or_default())
                        .attr("maxlength", max)
                        .errors(errors.under(&path, label));
                    if *required {
                        field = field.required();
                    }
                    if *key == "name" {
                        field = field.wrapper_class("mb-3 sm:col-span-2");
                    }
                    body.push(&render(field)?);
                }
                if !self.candidates.is_empty() || self.likely {
                    body.push(&self.review(errors)?);
                }
            }
        }
        Ok(body)
    }

    /// The duplicate review Core answered: the likely matches, each usable
    /// instead, and the reason that creates anyway.
    fn review(&self, errors: &Errors) -> Result<Html> {
        let prefix = &self.prefix;
        let mut rows = Vec::new();
        for (index, c) in self.candidates.iter().enumerate() {
            rows.push(CandidateRow {
                id: format!("{prefix}-candidate-{index}"),
                name: c.name.clone(),
                status: c.status.clone(),
                score: c.score.clone(),
                button: render(Button {
                    id: &format!("{prefix}-candidate-{index}-use"),
                    button_type: "button",
                    event: "use_candidate",
                    name: "company",
                    value: &c.id.to_string(),
                    body: Html::text("Use this company"),
                    ..Button::default()
                })?,
            });
        }
        let mut body = render(CandidateList {
            id: format!("{prefix}-candidates"),
            intro: if self.candidates.is_empty() {
                "Companies already has likely matches you may not list. Give a reason to create this company anyway, or ask an administrator."
            } else {
                "Companies already has these likely matches. Use one, or give a reason to create this company anyway."
            },
            rows,
        })?;
        body.push(&render(
            Field::in_form(self.form, "company_reason")
                .id(format!("{prefix}-company-reason"))
                .label("Reason to create anyway")
                .kind(InputKind::Text)
                .value(&self.reason)
                .attr("maxlength", "500")
                .errors(errors.under("company_reason", "Reason"))
                .wrapper_class("mb-3 sm:col-span-2"),
        )?);
        render(Wrapped {
            class: "sm:col-span-2",
            body,
        })
    }

    /// The company to link: the chosen one, or the one Core creates now from
    /// the new company's fields after its duplicate review. `event` is the
    /// page event the person raised, which the page declares for the Core
    /// contracts this may call.
    pub(super) async fn company(
        &mut self,
        pages: &Pages,
        scope: &Scope,
        credential: &str,
        event: &str,
    ) -> Result<Chosen> {
        if let Some(created) = self.created {
            return Ok(Chosen::Company(created));
        }
        match self.mode {
            Mode::Existing => Ok(match &self.selected {
                Some(selected) => match selected.id.parse::<i64>() {
                    Ok(id) => Chosen::Company(id),
                    Err(_) => Chosen::Invalid(errors_of("company", "is not a company")),
                },
                None => Chosen::Invalid(errors_of("company", "can't be blank")),
            }),
            Mode::New => self.create(pages, scope, credential, event).await,
        }
    }

    async fn create(
        &mut self,
        pages: &Pages,
        scope: &Scope,
        credential: &str,
        event: &str,
    ) -> Result<Chosen> {
        let mut input = json!({});
        for (key, ..) in NEW_FIELDS {
            f::put(
                &mut input,
                key,
                self.new.get(*key).map_or("", String::as_str),
            );
        }
        let reviewing = !self.candidates.is_empty() || self.likely;
        if reviewing {
            if self.reason.is_empty() {
                return Ok(Chosen::Invalid(errors_of(
                    "company_reason",
                    "can't be blank",
                )));
            }
            input["create_anyway"] = json!({
                "candidate_ids": self.candidates.iter().map(|c| c.id).collect::<Vec<_>>(),
                "reason": self.reason,
            });
        }
        let (code, review) = match pages
            .review_core(
                scope,
                credential,
                event,
                "core.company.create",
                input.clone(),
            )
            .await?
        {
            Reviewed::Valid { code, review } => (code, review),
            Reviewed::Invalid(fields) => return Ok(Chosen::Invalid(company_errors(fields))),
            Reviewed::Refused(message) => {
                return Ok(Chosen::Refused(super::worded(
                    message,
                    "You do not have permission to create companies.",
                )))
            }
        };
        if !reviewing {
            let candidates: Vec<Candidate> = review["duplicate_candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some(Candidate {
                        id: c["id"].as_i64()?,
                        name: c["name"].as_str()?.to_owned(),
                        status: c["status"].as_str().unwrap_or("").to_owned(),
                        score: c["score"]
                            .as_f64()
                            .map(|s| format!("{:.0}%", s * 100.0))
                            .unwrap_or_default(),
                    })
                })
                .collect();
            let likely = review["duplicates_likely"].as_bool().unwrap_or(false);
            if !candidates.is_empty() || likely {
                self.candidates = candidates;
                self.likely = likely;
                return Ok(Chosen::Review);
            }
        }
        match pages
            .commit_core(
                scope,
                credential,
                event,
                "core.company.create",
                &code,
                input,
            )
            .await?
        {
            Written::Done(record) => {
                let id = record["id"].as_i64().ok_or(Error::Internal)?;
                self.created = Some(id);
                self.candidates.clear();
                self.likely = false;
                self.choose(pages, scope, &id.to_string()).await?;
                Ok(Chosen::Company(id))
            }
            Written::Invalid(fields) => Ok(Chosen::Invalid(company_errors(fields))),
            Written::Refused(message) => Ok(Chosen::Refused(message)),
        }
    }
}

/// Whether the current company records `target` as its customer now.
pub(super) async fn relation(pages: &Pages, scope: &Scope, target: i64) -> Result<Relation> {
    let Some(company) = Pages::company(scope) else {
        return Ok(Relation::Hidden);
    };
    let links = match pages
        .read_core(
            scope,
            "core.company.relationship.list",
            json!({"company_id": company}),
        )
        .await
    {
        Ok(links) => links,
        Err(Error::Forbidden | Error::Unauthorized) => return Ok(Relation::Hidden),
        Err(error) => return Err(error),
    };
    Ok(links
        .as_array()
        .into_iter()
        .flatten()
        .find(|link| {
            link["direction"] == "outgoing"
                && link["role_code"] == "customer"
                && link["current"] == true
                && link["other"]["kind"] == "organisation"
                && link["other"]["id"].as_i64() == Some(target)
        })
        .map_or(Relation::Missing, |link| Relation::Recorded {
            since: link["dates"]["effective_from"].as_str().map(str::to_owned),
        }))
}

/// Records `target` as a customer of the current company through Core's
/// reviewed write, unless Core already records it. Answers why it could not.
pub(super) async fn record_relationship(
    pages: &Pages,
    scope: &Scope,
    credential: &str,
    event: &str,
    target: i64,
) -> Result<std::result::Result<(), String>> {
    let Some(company) = Pages::company(scope) else {
        return Ok(Err(super::DENIED.into()));
    };
    if let Relation::Recorded { .. } = relation(pages, scope, target).await? {
        return Ok(Ok(()));
    }
    let roles = match pages
        .read_core(scope, "core.company.relationship_role.list", json!({}))
        .await
    {
        Ok(roles) => roles,
        Err(Error::Forbidden | Error::Unauthorized) => {
            return Ok(Err(
                "you need permission to list companies to find the customer role".into(),
            ))
        }
        Err(error) => return Err(error),
    };
    let Some(role) = roles
        .as_array()
        .into_iter()
        .flatten()
        .find(|role| role["code"] == "customer")
        .and_then(|role| role["id"].as_i64())
    else {
        return Ok(Err(
            "the relationship roles have no customer role; add one under Companies › Relationship Roles".into(),
        ));
    };
    match pages
        .write_core(
            scope,
            credential,
            event,
            "core.company.relationship.assign",
            json!({
                "source_organisation_id": company,
                "target_kind": "organisation",
                "target_id": target,
                "role_id": role,
            }),
        )
        .await?
    {
        Written::Done(_) => Ok(Ok(())),
        Written::Invalid(fields) => Ok(Err(f::sentences(&fields))),
        Written::Refused(message) => Ok(Err(super::worded(
            message,
            "you do not have permission to change the current company's relationships",
        ))),
    }
}

/// The flash after a customer was added or linked, saying whether Core also
/// records the relationship.
pub(super) fn relationship_flash(
    done: &str,
    recorded: &std::result::Result<(), String>,
) -> (StatusKind, String) {
    match recorded {
        Ok(()) => (
            StatusKind::Success,
            format!("{done} The customer relationship is recorded in Companies."),
        ),
        Err(reason) => (
            StatusKind::Warning,
            format!("{done} The customer relationship was not recorded: {reason}. Record it on the company's Relationships page."),
        ),
    }
}

fn pages_hits(hits: &[Hit]) -> Vec<bilibi_ui::PickerHit> {
    hits.iter()
        .map(|hit| bilibi_ui::PickerHit {
            value: format!("{}:{}", hit.kind, hit.id),
            label: hit.label.clone(),
            kind_label: "Company".into(),
        })
        .collect()
}

fn errors_of(path: &str, message: &str) -> Errors {
    Errors(BTreeMap::from([(
        path.to_owned(),
        vec![message.to_owned()],
    )]))
}

/// Core's field names on the dialog's company fields. The dialog has no code
/// field: Core derives the code from the name, so a taken code means a
/// company of that name already exists and should be chosen instead.
fn company_errors(fields: BTreeMap<String, Vec<String>>) -> Errors {
    Errors(
        fields
            .into_iter()
            .map(|(field, messages)| match field.as_str() {
                "create_anyway" => ("company_reason".to_owned(), messages),
                "base" => ("base".to_owned(), messages),
                "code" => (
                    "company_name".to_owned(),
                    vec!["is already a company's name in Companies; choose that company instead of creating it".to_owned()],
                ),
                other => (format!("company_{other}"), messages),
            })
            .collect(),
    )
}

#[derive(askama::Template)]
#[template(
    source = r#"<div class="{{ class }}">{{ body|safe }}</div>"#,
    ext = "html"
)]
struct Wrapped {
    class: &'static str,
    body: Html,
}

struct CandidateRow {
    id: String,
    name: String,
    status: String,
    score: String,
    button: Html,
}

#[derive(askama::Template)]
#[template(
    source = r#"<div id="{{ id }}" class="mb-3 rounded-md border border-line bg-surface-muted p-3 text-sm"><p class="text-ink-muted">{{ intro }}</p>{% if !rows.is_empty() %}<ul class="mt-2 space-y-2">{% for row in rows %}<li id="{{ row.id }}" class="flex flex-wrap items-center justify-between gap-2"><span><span class="font-medium text-ink-strong">{{ row.name }}</span> <span class="text-ink-muted">· {{ row.status }} · {{ row.score }} match</span></span>{{ row.button|safe }}</li>{% endfor %}</ul>{% endif %}</div>"#,
    ext = "html"
)]
struct CandidateList {
    id: String,
    intro: &'static str,
    rows: Vec<CandidateRow>,
}
