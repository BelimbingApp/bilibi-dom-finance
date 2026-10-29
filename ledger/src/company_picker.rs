//! Choosing the Core company a customer is, shared by Add customer and Link
//! to company: an existing company found by name through Base Records and
//! the shared `RecordPicker`, or a new one whose duplicate review the dialog
//! shows before the person creates anyway with a reason.
//!
//! `submit_composed` reviews and commits the customer's single operation
//! (company, Finance link and relationship in one transaction), including
//! Core's duplicate suggestions. The vendor's existing `company` flow still
//! creates the company through Core's reviewed `core.company.create` and
//! records the relationship through `core.company.relationship.assign` as
//! separate writes, and reports a later refusal.
use askama::Template;
use bilibi_agent_api::{
    page::{Call, PageContracts},
    Error as ContractError,
};
use bilibi_database::{Error, Result};
use bilibi_records::{Hit, Reference, Resolved};
use bilibi_tenancy::Scope;
use bilibi_ui::{error_sentence, ErrorSummary};
use bilibi_ui::{ActionLink, Button, Field, Html, InputKind, RecordPicker, StatusKind};
use serde_json::json;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

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
pub enum Mode {
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
pub enum Chosen {
    /// The company to link, proven by Core.
    Company(i64),
    /// CompanyErrors to show on the dialog's company fields.
    Invalid(CompanyErrors),
    Refused(String),
    /// Likely duplicates were found; the dialog now shows them and waits
    /// for a choice or a reason.
    Review,
}

/// The result of a single composed customer operation, including its duplicate review.
pub enum CompanySubmission {
    Done(Value),
    Invalid(CompanyErrors),
    Refused(String),
    Review,
}

/// Whether the current company records the customer relationship.
pub enum Relation {
    Recorded {
        since: Option<String>,
    },
    Missing,
    /// The reader may not read relationships.
    Hidden,
}

pub struct CompanyChooser {
    prefix: String,
    form: &'static str,
    pub mode: Mode,
    query: String,
    hits: Vec<Hit>,
    existing: HashMap<String, String>,
    existing_label: &'static str,
    pub selected: Option<Resolved>,
    /// The chosen company's own code, offered as the Finance code until the
    /// person types another; `None` when Core hides it from the reader.
    code: Option<String>,
    offered: Option<String>,
    new: HashMap<String, String>,
    candidates: Vec<Candidate>,
    likely: bool,
    reason: String,
    /// The company this dialog created and still has chosen, for the page's
    /// warning that it exists while the customer was not added.
    pub created: Option<i64>,
}

impl CompanyChooser {
    pub fn new(prefix: &str, form: &'static str) -> Self {
        Self {
            prefix: prefix.into(),
            form,
            mode: Mode::Existing,
            query: String::new(),
            hits: Vec::new(),
            existing: HashMap::new(),
            existing_label: "",
            selected: None,
            code: None,
            offered: None,
            new: HashMap::new(),
            candidates: Vec::new(),
            likely: false,
            reason: String::new(),
            created: None,
        }
    }

    /// Mark companies already linked to this company's Finance records.
    pub fn existing(mut self, label: &'static str, records: HashMap<String, String>) -> Self {
        self.existing_label = label;
        self.existing = records;
        self
    }

    /// The posted fields `(path, dom id, label)`, for the error summary.
    pub fn labels(&self) -> Vec<(String, String, String)> {
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
    pub fn read(&mut self, form: &HashMap<String, String>) -> bool {
        if let Some(mode) = form.get("company_mode") {
            self.mode = if mode == "new" {
                self.created = None;
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

    pub async fn search(&mut self, pages: &CompanyPickerContext, scope: &Scope) -> Result<()> {
        self.hits = if self.query.trim().is_empty() {
            Vec::new()
        } else {
            pages.records.search(scope, &KINDS, &self.query).await?
        };
        Ok(())
    }

    /// The person chose a hit (`record=kind:id`); Base Records names it
    /// again with their permissions, so a forged value stays unnamed.
    pub async fn pick(
        &mut self,
        pages: &CompanyPickerContext,
        scope: &Scope,
        record: &str,
    ) -> Result<()> {
        let (kind, id) = record.split_once(':').ok_or(Error::Invalid)?;
        if kind != KINDS[0] {
            return Err(Error::Invalid);
        }
        if self.existing.contains_key(id) {
            return Err(Error::Invalid);
        }
        self.choose(pages, scope, id).await
    }

    async fn choose(
        &mut self,
        pages: &CompanyPickerContext,
        scope: &Scope,
        id: &str,
    ) -> Result<()> {
        let named = pages
            .records
            .resolve(scope, &[Reference::new(KINDS[0], id)])
            .await?;
        self.selected = named.into_iter().next();
        self.code = match id.parse::<i64>() {
            Ok(company) => pages
                .read_core(scope, "core.company.get", json!({"company_id": company}))
                .await
                .ok()
                .and_then(|company| company["code"].as_str().map(str::to_owned))
                .filter(|code| !code.is_empty()),
            Err(_) => None,
        };
        self.mode = Mode::Existing;
        self.hits.clear();
        self.query.clear();
        Ok(())
    }

    /// Offer the chosen company's code as the Finance `code`: it replaces a
    /// blank field or the code offered for the company chosen before, never
    /// what the person typed.
    pub fn offer_code(&mut self, values: &mut HashMap<String, String>) {
        let Some(code) = self.code.clone() else {
            return;
        };
        let current = values.get("code").map(String::as_str).unwrap_or_default();
        if current.is_empty() || self.offered.as_deref() == Some(current) {
            values.insert("code".to_owned(), code.clone());
            self.offered = Some(code);
        }
    }

    pub fn clear(&mut self) {
        self.selected = None;
        self.created = None;
        self.hits.clear();
        self.query.clear();
    }

    /// The person took a duplicate candidate instead of creating a company.
    pub async fn use_candidate(
        &mut self,
        pages: &CompanyPickerContext,
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
    pub fn render(&self, errors: &CompanyErrors, framed: bool) -> Result<Html> {
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
                .hint("Company details and billing addresses are read from Companies.")
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
                let available: Vec<_> = self
                    .hits
                    .iter()
                    .filter(|h| !self.existing.contains_key(&h.id))
                    .cloned()
                    .collect();
                let picker = RecordPicker::new(
                    &format!("{prefix}-company"),
                    self.form,
                    "company",
                    "Company",
                    &self.query,
                    &pages_hits(&available),
                    selected
                        .as_ref()
                        .map(|(value, pill)| (value.as_str(), pill.clone())),
                    ("pick_company", "clear_company"),
                    errors.under("company", "Company"),
                );
                // RecordPicker cannot render disabled hits with links yet. When
                // every hit is linked, draw its search field without its empty
                // message; the excluded hits follow as disabled controls.
                let mut choices =
                    if self.selected.is_none() && available.is_empty() && !self.hits.is_empty() {
                        render(
                            Field::in_form(self.form, "company_search")
                                .id(format!("{prefix}-company-search"))
                                .label("Company")
                                .kind(InputKind::Search)
                                .value(&self.query)
                                .attr("data-debounce", "300")
                                .autocomplete("off")
                                .hint("Type part of a name.")
                                .errors(errors.under("company", "Company")),
                        )?
                    } else {
                        render(picker)?
                    };
                if self.selected.is_none() {
                    for (index, hit) in self.hits.iter().enumerate() {
                        if let Some(href) = self.existing.get(&hit.id) {
                            choices.push(&render(Button {
                                id: &format!("{prefix}-existing-{index}"),
                                button_type: "button",
                                disabled: true,
                                class: "w-full min-w-0 justify-start",
                                body: Html::text(&format!(
                                    "{} · {}",
                                    hit.label, self.existing_label
                                )),
                                ..Button::default()
                            })?);
                            choices.push(&render(ActionLink {
                                href,
                                body: Html::text(if self.existing_label == "Already a customer" {
                                    "View customer"
                                } else {
                                    "View supplier"
                                }),
                                ..ActionLink::default()
                            })?);
                        }
                    }
                }
                body.push(&render(Wrapped {
                    class: "mb-3 sm:col-span-2",
                    body: choices,
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
    fn review(&self, errors: &CompanyErrors) -> Result<Html> {
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

    /// Review and commit the owner's single operation. The owner accepts
    /// either `core_company_id` or `new_company`, and exposes Core's duplicate
    /// suggestions under `changes.target.creation` without creating anything.
    pub async fn submit_composed(
        &mut self,
        pages: &CompanyPickerContext,
        scope: &Scope,
        credential: &str,
        event: &str,
        contract: &'static str,
        mut input: Value,
    ) -> Result<CompanySubmission> {
        input["company_id"] = json!(match CompanyPickerContext::company(scope) {
            Some(id) => id,
            None => return Ok(CompanySubmission::Refused(DENIED.into())),
        });
        let reviewing = !self.candidates.is_empty() || self.likely;
        match self.mode {
            Mode::Existing => match self
                .selected
                .as_ref()
                .and_then(|s| s.id.parse::<i64>().ok())
            {
                Some(id) => input["core_company_id"] = json!(id),
                None => {
                    return Ok(CompanySubmission::Invalid(errors_of(
                        "company",
                        "can't be blank",
                    )));
                }
            },
            Mode::New => {
                let mut new = json!({});
                for (key, ..) in NEW_FIELDS {
                    put(&mut new, key, self.new.get(*key).map_or("", String::as_str));
                }
                if reviewing {
                    if self.reason.is_empty() {
                        return Ok(CompanySubmission::Invalid(errors_of(
                            "company_reason",
                            "can't be blank",
                        )));
                    }
                    new["create_anyway"] = json!({"candidate_ids":self.candidates.iter().map(|c| c.id).collect::<Vec<_>>(),"reason":self.reason});
                }
                input["new_company"] = new;
            }
        }
        let (code, review) = match pages
            .review_core(scope, credential, event, contract, input.clone())
            .await?
        {
            Reviewed::Valid { code, review } => (code, review),
            Reviewed::Invalid(fields) => {
                return Ok(CompanySubmission::Invalid(composed_errors(fields)));
            }
            Reviewed::Refused(message) => return Ok(CompanySubmission::Refused(message)),
        };
        let creation = &review["changes"]["target"]["creation"];
        let candidates: Vec<Candidate> = creation["duplicate_candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                Some(Candidate {
                    id: c["id"].as_i64()?,
                    name: c["name"].as_str()?.to_owned(),
                    status: c["status"].as_str().unwrap_or("").into(),
                    score: c["score"]
                        .as_f64()
                        .map(|s| format!("{:.0}%", s * 100.0))
                        .unwrap_or_default(),
                })
            })
            .collect();
        let likely = creation["duplicates_likely"].as_bool().unwrap_or(false);
        let changed = candidates.iter().map(|c| c.id).collect::<Vec<_>>()
            != self.candidates.iter().map(|c| c.id).collect::<Vec<_>>();
        if (!reviewing || changed) && (!candidates.is_empty() || likely) {
            self.candidates = candidates;
            self.likely = likely;
            return Ok(CompanySubmission::Review);
        }
        Ok(
            match pages
                .commit_core(scope, credential, event, contract, &code, input)
                .await?
            {
                Written::Done(record) => CompanySubmission::Done(record),
                Written::Invalid(fields) => CompanySubmission::Invalid(composed_errors(fields)),
                Written::Refused(message) => CompanySubmission::Refused(message),
            },
        )
    }

    /// The company to link: the chosen one, or the one Core creates now from
    /// the new company's fields after its duplicate review. `event` is the
    /// page event the person raised, which the page declares for the Core
    /// contracts this may call.
    pub async fn company(
        &mut self,
        pages: &CompanyPickerContext,
        scope: &Scope,
        credential: &str,
        event: &str,
    ) -> Result<Chosen> {
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
        pages: &CompanyPickerContext,
        scope: &Scope,
        credential: &str,
        event: &str,
    ) -> Result<Chosen> {
        let mut input = json!({});
        for (key, ..) in NEW_FIELDS {
            put(
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
                return Ok(Chosen::Refused(worded(
                    message,
                    "You do not have permission to create companies.",
                )));
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
pub async fn relation(
    pages: &CompanyPickerContext,
    scope: &Scope,
    target: i64,
) -> Result<Relation> {
    let Some(company) = CompanyPickerContext::company(scope) else {
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
                && link["role_code"] == pages.role
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
pub async fn record_relationship(
    pages: &CompanyPickerContext,
    scope: &Scope,
    credential: &str,
    event: &str,
    target: i64,
) -> Result<std::result::Result<(), String>> {
    let Some(company) = CompanyPickerContext::company(scope) else {
        return Ok(Err(DENIED.into()));
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
            return Ok(Err(format!(
                "you need permission to list companies to find the {} role",
                pages.role
            )));
        }
        Err(error) => return Err(error),
    };
    let Some(role) = roles
        .as_array()
        .into_iter()
        .flatten()
        .find(|role| role["code"] == pages.role)
        .and_then(|role| role["id"].as_i64())
    else {
        return Ok(Err(format!(
            "the relationship roles have no {} role; add one under Companies › Relationship Roles",
            pages.role
        )));
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
        Written::Invalid(fields) => Ok(Err(sentences(&fields))),
        Written::Refused(message) => Ok(Err(worded(
            message,
            "you do not have permission to change the current company's relationships",
        ))),
    }
}

/// The flash after a customer was added or linked, saying whether Core also
/// records the relationship.
pub fn relationship_flash(
    role: &str,
    done: &str,
    recorded: &std::result::Result<(), String>,
) -> (StatusKind, String) {
    match recorded {
        Ok(()) => (
            StatusKind::Success,
            format!("{done} The {role} relationship is recorded in Companies."),
        ),
        Err(reason) => (
            StatusKind::Warning,
            format!(
                "{done} The {role} relationship was not recorded: {reason}. Record it on the company's Relationships page."
            ),
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

fn errors_of(path: &str, message: &str) -> CompanyErrors {
    CompanyErrors(BTreeMap::from([(
        path.to_owned(),
        vec![message.to_owned()],
    )]))
}

/// Core's field names on the dialog's company fields. The dialog has no code
/// field: Core derives the code from the name, so a taken code means a
/// company of that name already exists and should be chosen instead.
fn composed_errors(fields: BTreeMap<String, Vec<String>>) -> CompanyErrors {
    CompanyErrors(
        fields
            .into_iter()
            .map(|(field, messages)| {
                let key = if field == "core_company_id" || field == "new_company" {
                    "company".to_owned()
                } else if let Some(field) = field.strip_prefix("new_company.") {
                    if field == "create_anyway" {
                        "company_reason".to_owned()
                    } else if field == "code" {
                        "company_name".to_owned()
                    } else {
                        format!("company_{field}")
                    }
                } else {
                    field
                };
                (key, messages)
            })
            .collect(),
    )
}

fn company_errors(fields: BTreeMap<String, Vec<String>>) -> CompanyErrors {
    CompanyErrors(
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

/// Shared Core company picker. Each page supplies its own declared service
/// and relationship role; every Core write uses that page's live event.
#[derive(Clone)]
pub struct CompanyPickerContext {
    pub records: Arc<bilibi_records::Records>,
    pub contracts: Arc<PageContracts>,
    pub service: &'static str,
    pub role: &'static str,
}
impl CompanyPickerContext {
    fn company(scope: &Scope) -> Option<i64> {
        scope.current_company().map(|id| id.get())
    }
    async fn read_core(&self, scope: &Scope, key: &str, input: Value) -> Result<Value> {
        let mut answer = self
            .contracts
            .read(scope, key, &input)
            .await
            .map_err(read_error)?;
        Ok(answer["result"].take())
    }
    pub fn call<'a>(
        &self,
        scope: &'a Scope,
        credential: &'a str,
        event: &'a str,
        contract: &'static str,
    ) -> Call<'a> {
        Call {
            scope,
            credential,
            service: self.service,
            event,
            contract,
        }
    }

    /// A reviewed write of another module's contract on the person's behalf,
    /// with the input as given: Core names its own company, so none is added.
    pub async fn write_core(
        &self,
        scope: &Scope,
        credential: &str,
        event: &str,
        contract: &'static str,
        input: Value,
    ) -> Result<Written> {
        let answer = self
            .contracts
            .submit(&self.call(scope, credential, event, contract), input)
            .await;
        Self::written(contract, event, answer)
    }

    /// The review half of a confirmation on another module's contract: the
    /// owner's review and the code that commits exactly that input.
    pub async fn review_core(
        &self,
        scope: &Scope,
        credential: &str,
        event: &str,
        contract: &'static str,
        input: Value,
    ) -> Result<Reviewed> {
        match self
            .contracts
            .preview(&self.call(scope, credential, event, contract), input)
            .await
        {
            Ok(value) => {
                let result = &value["results"][0];
                if result["valid"].as_bool() == Some(true) {
                    if let Some(code) = value["plan_code"].as_str() {
                        return Ok(Reviewed::Valid {
                            code: code.to_owned(),
                            review: result["preview"]["result"].clone(),
                        });
                    }
                }
                let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
                if let Some(map) = result["error"]["fields"].as_object() {
                    for (field, messages) in map {
                        fields.insert(
                            field.clone(),
                            messages
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|m| m.as_str().map(str::to_owned))
                                .collect(),
                        );
                    }
                }
                if fields.is_empty() {
                    return Ok(Reviewed::Refused(
                        if result["error"]["code"].as_str() == Some("missing_capability") {
                            DENIED.into()
                        } else {
                            "That could not be reviewed. Please try again.".into()
                        },
                    ));
                }
                Ok(Reviewed::Invalid(fields))
            }
            Err(error) => Ok(match Self::written(contract, event, Err(error))? {
                Written::Invalid(fields) => Reviewed::Invalid(fields),
                Written::Refused(message) => Reviewed::Refused(message),
                Written::Done(_) => Reviewed::Refused(DENIED.into()),
            }),
        }
    }

    /// Commits a reviewed confirmation with the same input and its code. A
    /// commit answers the plan's envelope (`results[0].result`), where a
    /// submit answers the one result itself.
    pub async fn commit_core(
        &self,
        scope: &Scope,
        credential: &str,
        event: &str,
        contract: &'static str,
        code: &str,
        input: Value,
    ) -> Result<Written> {
        let answer = self
            .contracts
            .commit(&self.call(scope, credential, event, contract), code, input)
            .await
            .map(|mut value| value["results"][0]["result"].take());
        Self::written(contract, event, answer)
    }

    fn written(
        contract: &str,
        event: &str,
        answer: std::result::Result<Value, ContractError>,
    ) -> Result<Written> {
        match answer {
            Ok(mut answer) => Ok(Written::Done(answer["result"]["record"].take())),
            Err(ContractError::InvalidInput(fields)) => Ok(Written::Invalid(fields)),
            Err(ContractError::Forbidden | ContractError::Backend(Error::Forbidden)) => {
                Ok(Written::Refused(DENIED.into()))
            }
            Err(ContractError::StalePlan | ContractError::PlanExpired) => Ok(Written::Refused(
                "The details changed or this review expired. Review them again before saving."
                    .into(),
            )),
            Err(ContractError::NotFound | ContractError::Backend(Error::NotFound)) => Ok(
                Written::Refused("That record no longer exists. Reload the page.".into()),
            ),
            // The host signs the browser in again, or sends it to confirm.
            Err(ContractError::Backend(
                error @ (Error::Unauthorized | Error::StepUpRequired { .. }),
            )) => Err(error),
            Err(ContractError::Backend(Error::Conflict)) => Ok(Written::Refused(
                "Someone changed this while you were working. Reload the page and try again."
                    .into(),
            )),
            Err(error) => {
                eprintln!("receivables page: {contract} from {event} failed: {error:?}");
                Ok(Written::Refused(
                    "That could not be saved. Please try again.".into(),
                ))
            }
        }
    }
}
/// A refusal for a missing permission, worded for what the person was doing.
pub fn worded(message: String, denied: &str) -> String {
    if message == DENIED {
        denied.into()
    } else {
        message
    }
}

pub const DENIED: &str = "You do not have permission to do that.";

/// What a write came to.
pub enum Written {
    Done(Value),
    /// Field errors keyed by the input's path (`lines.0.quantity`, `base`).
    Invalid(BTreeMap<String, Vec<String>>),
    Refused(String),
}

/// What a review came to: the owner's review with the code that commits
/// it, or the refusal.
pub enum Reviewed {
    Valid { code: String, review: Value },
    Invalid(BTreeMap<String, Vec<String>>),
    Refused(String),
}

fn read_error(error: ContractError) -> Error {
    match error {
        ContractError::Backend(error) => error,
        ContractError::Forbidden => Error::Forbidden,
        ContractError::NotFound => Error::NotFound,
        ContractError::InvalidInput(_) => Error::Invalid,
        _ => Error::Internal,
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompanyErrors(pub BTreeMap<String, Vec<String>>);

impl CompanyErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The messages for one input field.
    pub fn of(&self, path: &str) -> Vec<String> {
        self.0.get(path).cloned().unwrap_or_default()
    }

    /// The messages for one input field as sentences that name it, the way
    /// the summary reads them, for the line under the field.
    pub fn under(&self, path: &str, label: &str) -> Vec<String> {
        self.of(path)
            .iter()
            .map(|message| error_sentence(label, message))
            .collect()
    }

    /// The messages that belong to no field (a refusal of the whole request).
    pub fn base(&self) -> Vec<String> {
        self.of("base")
    }

    /// Everything the form was refused for, each sentence linked to the field
    /// that holds it. `labels` names `(path, dom id, label)` for the fields on
    /// the form; any other path is shown by its own words.
    pub fn summary(&self, id: &str, labels: &[(String, String, String)]) -> ErrorSummary {
        let mut summary = ErrorSummary::new(id);
        for (path, messages) in &self.0 {
            let known = labels.iter().find(|(p, _, _)| p == path);
            for message in messages {
                match known {
                    Some((_, dom, label)) => summary.push(dom, error_sentence(label, message)),
                    None => summary.push("", message.clone()),
                }
            }
        }
        summary
    }
}

fn render(fragment: impl Template) -> Result<Html> {
    Html::render(&fragment).map_err(|_| Error::Internal)
}
fn put(input: &mut Value, key: &str, text: &str) {
    if !text.is_empty() {
        input[key] = json!(text);
    }
}
fn sentences(fields: &BTreeMap<String, Vec<String>>) -> String {
    fields
        .iter()
        .flat_map(|(k, v)| v.iter().map(move |m| error_sentence(k, m)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The company's billing address as of `today`, from Core Address's owner
/// list: the attachment carrying the billing kind within its dates, primary
/// first (the list's order), else the primary address, else the first.
pub fn company_billing_lines(rows: &Value, today: &str) -> Vec<String> {
    let attachment = |row: &Value| row["attachment"]["attachment"].clone();
    let current: Vec<&Value> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            let dates = &row["attachment"]["attachment"]["dates"];
            let from = dates["effective_from"].as_str();
            let to = dates["effective_to"].as_str();
            today.is_empty() || (from.is_none_or(|f| f <= today) && to.is_none_or(|t| t >= today))
        })
        .collect();
    let billing = |row: &Value| {
        attachment(row)["kinds"]
            .as_array()
            .is_some_and(|k| k.iter().any(|v| v == "billing"))
    };
    let primary = |row: &Value| attachment(row)["is_primary"] == true;
    let chosen = current
        .iter()
        .find(|row| billing(row))
        .or_else(|| current.iter().find(|row| primary(row)))
        .or_else(|| current.first());
    let Some(row) = chosen else {
        return Vec::new();
    };
    let a = &row["address"]["address"];
    let text = |key: &str| a[key].as_str().map(str::trim).filter(|s| !s.is_empty());
    let mut lines: Vec<String> = ["line1", "line2", "line3"]
        .into_iter()
        .filter_map(text)
        .map(str::to_owned)
        .collect();
    let place: Vec<&str> = ["postcode", "locality", "admin1_code", "country_iso"]
        .into_iter()
        .filter_map(text)
        .collect();
    if !place.is_empty() {
        lines.push(place.join(" "));
    }
    lines
}
