//! Bilimbi Finance.Ledger.Journal, adapted to Workflow’s shared approval gate. Ledger alone keeps
//! their immutable submissions, final decisions, posting and correction.
use super::*;
use bilibi_module_registry::{Contribution, Provenance, RegistryError};
use bilibi_workflow::{
    ApprovalAccess, ApprovalAdapter, ApprovalCase, ApprovalDecisionAction, ApprovalFact,
    ApprovalFactKind, ApprovalIdentities, ApprovalMode, ApprovalOutcome, ApprovalPolicy,
    ApprovalSeat, ApprovalSelector, ApprovalStage, ApprovalSubject, ApprovalSummary, Engine, Load,
    Operation, OwnerFuture, Subject, SubjectAdapter, CONTRIBUTIONS,
};
use std::{collections::BTreeMap, sync::OnceLock};
use tokio_postgres::Transaction;

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
const SUBJECT: &str = "finance.ledger.journal";
pub(crate) const CASE: &str = "finance.ledger.manual_journal";
const ACTION: &str = "finance.ledger.journal.decide";

pub(crate) fn engine() -> Result<&'static Engine> {
    ENGINE
        .get()
        .map(Arc::as_ref)
        .ok_or(LedgerError::Failed(Error::Internal))
}
pub(crate) fn bind(engine: Arc<Engine>) -> bilibi_module_registry::Result<()> {
    engine.bind_approval_case(MODULE, CASE, Arc::new(Owner))?;
    contracts::bind(engine.clone())?;
    ENGINE
        .set(engine)
        .map_err(|_| RegistryError::Service("duplicate journal approval binding".into()))
}
fn policy(case: &ApprovalCase) -> ApprovalPolicy {
    ApprovalPolicy {
        stages: vec![ApprovalStage {
            key: "review".into(),
            label: "Journal approval".into(),
            mode: ApprovalMode::Any,
            seats: vec![ApprovalSeat {
                key: "approver".into(),
                label: "Approver".into(),
                capability: case.decisions[0].capability.into(),
                selector: ApprovalSelector::Capability,
                delegation: case.delegation,
                reminder_hours: Some(24),
                escalation_hours: None,
                escalation: None,
            }],
        }],
        rules: vec![],
        distinct_stages: false,
        approval_reason: false,
        allow_preparer: false,
    }
}
const DECLARATION: ApprovalCase = ApprovalCase {
    key: CASE,
    label: "Manual journals",
    subject: SUBJECT,
    process: CASE,
    default_policy: policy,
    decisions: &[ApprovalDecisionAction {
        label: "Decide a manual journal",
        key: ACTION,
        capability: APPROVE_JOURNALS,
        contract: "finance.ledger.journal.approval.decide",
    }],
    recall_capability: SUBMIT_JOURNALS,
    delegation_capability: APPROVE_JOURNALS,
    policy_capability: MANAGE_DUTY_SEPARATION,
    review_capability: REVIEW_EXCEPTIONS,
    policy_contract: "finance.ledger.journal.approval.policy",
    recall_contract: "finance.ledger.journal.approval.recall",
    delegation_contract: "finance.ledger.journal.approval.delegate",
    revoke_contract: "finance.ledger.journal.approval.revoke",
    review_contract: "finance.ledger.journal.approval.review",
    facts: &[("amount", ApprovalFactKind::Money)],
    relationships: &[],
    required_capabilities: &[APPROVE_JOURNALS],
    distinct_stages: false,
    approval_reason: false,
    reason_characters: 500,
    reason_bytes: 2000,
    allow_preparer: true,
    delegation: true,
    forbid_impersonation: false,
    step_up: None,
};
bilibi_workflow::declare_approval_case!(contracts,"finance/ledger",DECLARATION,
    policy:"finance.ledger.journal.approval.policy",recall:"finance.ledger.journal.approval.recall",
    delegation:"finance.ledger.journal.approval.delegate",revoke:"finance.ledger.journal.approval.revoke",
    review:"finance.ledger.journal.approval.review",step_up:&[],decision_input:||json!({"type":"object","properties":{}}),
    decisions:[(DECIDE,"finance.ledger.journal.approval.decide","finance.ledger.journal.decide","finance.ledger.journals.approve")]);
struct Owner;
static JOURNAL_OWNER: Owner = Owner;
inventory::submit! { Contribution::new(CONTRIBUTIONS, SUBJECT, Provenance {module:MODULE,source:file!(),line:line!()},
&bilibi_workflow::Contribution::Subject {key:SUBJECT,aliases:&[],adapter:&JOURNAL_OWNER}) }

fn db_error(error: LedgerError) -> Error {
    match error {
        LedgerError::Failed(error) => error,
        LedgerError::Unauthorized => Error::Forbidden,
        LedgerError::CompanyNotFound
        | LedgerError::JournalNotFound
        | LedgerError::ExceptionNotFound => Error::NotFound,
        LedgerError::Invalid(fields) => Error::Refused(
            fields
                .iter()
                .map(|f| format!("{}: {}", f.field, f.message))
                .collect::<Vec<_>>()
                .join("; "),
        ),
        error => Error::Refused(error.to_string()),
    }
}
fn refusal(error: Error) -> LedgerError {
    match error {
        Error::NotFound=>LedgerError::ApprovalNotConfigured,
        Error::Forbidden=>LedgerError::ApprovalRefused("These approval rules require another eligible person. Open Waiting for my approval to see the decisions you can make.".into()),
        Error::Conflict=>LedgerError::ApprovalRefused("The approval changed. Reload the journal before deciding.".into()),
        Error::Refused(message)=>LedgerError::ApprovalRefused(message),
        error=>LedgerError::Failed(error),
    }
}
async fn journal_company(
    sql: &Transaction<'_>,
    scope: &Scope,
    id: i64,
) -> bilibi_database::Result<i64> {
    Ok(sql
        .query_opt(
            "SELECT company_id FROM finance_ledger_journals WHERE tenant_id=$1 AND id=$2",
            &[&scope.tenant(), &id],
        )
        .await?
        .ok_or(Error::NotFound)?
        .get(0))
}
fn approval_facts(journal: Journal, book: Book, company: i64) -> ApprovalSubject {
    let mut preparers = ApprovalIdentities::default();
    preparers.accounts.insert(journal.submitted_by_user_id);
    if let Some(id) = journal.submitted_by_impersonator_id {
        preparers.accounts.insert(id);
    }
    let amount: rust_decimal::Decimal = journal.lines.iter().map(|l| l.debit.amount()).sum();
    ApprovalSubject {
        company,
        revision: format!("journal:{}", journal.id),
        facts: BTreeMap::from([(
            "amount".into(),
            ApprovalFact::Money {
                currency: book.currency_code.clone(),
                value: amount.to_string(),
            },
        )]),
        preparers,
        beneficiaries: Default::default(),
        label: format!("Journal {}", journal.journal_number),
        url: format!("/finance/ledger/journals/{}", journal.id),
        summary: vec![
            ApprovalSummary {
                label: "Description".into(),
                value: journal.description,
            },
            ApprovalSummary {
                label: "Amount".into(),
                value: format!(
                    "{} {}",
                    book.currency_code,
                    Money::display_decimal(amount, book.minor_units)
                ),
            },
        ],
    }
}
impl SubjectAdapter for Owner {
    fn maintenance_load<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a bilibi_authz::SystemScope,
        id: i64,
    ) -> OwnerFuture<'a, Subject> {
        Box::pin(async move {
            let company:i64=sql.query_opt("SELECT company_id FROM finance_ledger_journals WHERE tenant_id=$1 AND id=$2 FOR UPDATE",&[&scope.tenant(),&id]).await?.ok_or(Error::NotFound)?.get(0);
            let journal = journals::find(sql, scope.tenant(), company, id)
                .await?
                .ok_or(Error::NotFound)?;
            Ok(Subject {
                id,
                tenant: scope.tenant(),
                status: journal.status.as_str().to_owned(),
                version: i64::from(journal.status != JournalStatus::Submitted),
                facts: json!({"company":company}),
            })
        })
    }
    fn load<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        id: i64,
        mode: Load,
    ) -> OwnerFuture<'a, Subject> {
        Box::pin(async move {
            let company = journal_company(sql, scope, id).await?;
            authorize(sql, scope, company, VIEW)
                .await
                .map_err(db_error)?;
            if matches!(mode, Load::Lock) {
                bilibi_company::require_writable(sql, scope, company, true)
                    .await
                    .map_err(|e| match e {
                        WriteError::NotFound => Error::NotFound,
                        WriteError::Archived => {
                            Error::Refused(LedgerError::CompanyArchived.to_string())
                        }
                        WriteError::Access(e) => e,
                        _ => Error::Invalid,
                    })?;
            }
            let journal = journals::find(sql, scope.tenant(), company, id)
                .await?
                .ok_or(Error::NotFound)?;
            let status = journal.status.as_str().to_owned();
            Ok(Subject {
                id,
                tenant: scope.tenant(),
                version: i64::from(status != "submitted"),
                status,
                facts: json!({"company":company}),
            })
        })
    }
    fn authorize<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        subject: &'a Subject,
        operation: Operation,
    ) -> OwnerFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(
                operation,
                Operation::Read
                    | Operation::ReadRun
                    | Operation::StartRun
                    | Operation::ExecuteAction
            ) {
                return Err(Error::Forbidden);
            }
            let company = subject.facts["company"].as_i64().ok_or(Error::Invalid)?;
            authorize(sql, scope, company, VIEW).await.map_err(db_error)
        })
    }
    fn persist<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        _: &'a Scope,
        _: &'a Subject,
        _: &'a str,
        _: &'a bilibi_workflow::Request,
    ) -> OwnerFuture<'a, Vec<Change>> {
        Box::pin(async { Err(Error::Forbidden) })
    }
}
impl ApprovalAdapter for Owner {
    fn authorize_company<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        company: i64,
        access: ApprovalAccess,
    ) -> OwnerFuture<'a, ()> {
        Box::pin(async move {
            let cap = match access {
                ApprovalAccess::Policy => MANAGE_DUTY_SEPARATION,
                ApprovalAccess::Submit | ApprovalAccess::Recall => SUBMIT_JOURNALS,
                ApprovalAccess::Decide | ApprovalAccess::Delegate => APPROVE_JOURNALS,
                ApprovalAccess::Review => REVIEW_EXCEPTIONS,
                ApprovalAccess::Read => VIEW,
            };
            authorize(sql, scope, company, cap)
                .await
                .map_err(db_error)?;
            if !matches!(access, ApprovalAccess::Read) {
                bilibi_company::require_writable(sql, scope, company, true)
                    .await
                    .map_err(|e| match e {
                        WriteError::NotFound => Error::NotFound,
                        WriteError::Archived => {
                            Error::Refused(LedgerError::CompanyArchived.to_string())
                        }
                        WriteError::Access(e) => e,
                        _ => Error::Invalid,
                    })?;
            }
            Ok(())
        })
    }
    fn inspect<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        subject: &'a Subject,
        access: ApprovalAccess,
    ) -> OwnerFuture<'a, ApprovalSubject> {
        Box::pin(async move {
            let company = subject.facts["company"].as_i64().ok_or(Error::Invalid)?;
            self.authorize_company(sql, scope, company, access).await?;
            let journal = journals::find(sql, scope.tenant(), company, subject.id)
                .await?
                .ok_or(Error::NotFound)?;
            if matches!(
                access,
                ApprovalAccess::Submit | ApprovalAccess::Decide | ApprovalAccess::Recall
            ) && journal.status != JournalStatus::Submitted
            {
                return Err(Error::Conflict);
            }
            if matches!(access, ApprovalAccess::Recall)
                && journal.submitted_by_user_id != scope.user()
            {
                return Err(db_error(LedgerError::NotSubmitter));
            }
            let book = books::find(sql, scope.tenant(), company)
                .await?
                .ok_or(Error::NotFound)?;
            Ok(approval_facts(journal, book, company))
        })
    }
    fn cover_candidate<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        company: i64,
        account: i64,
        identity: &'a bilibi_tenancy::AccountIdentity,
    ) -> OwnerFuture<'a, bool> {
        Box::pin(async move {
            Ok(identity.organisation.is_some_and(|c| c.get() == company)
                || bilibi_authz::effective_capabilities_of(sql, scope, account)
                    .await?
                    .allowed
                    .contains("admin.company.tenant-wide.manage"))
        })
    }
    fn candidate<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a Scope,
        subject: &'a Subject,
        company: i64,
        account: i64,
        identity: &'a bilibi_tenancy::AccountIdentity,
    ) -> OwnerFuture<'a, bool> {
        Box::pin(async move {
            Ok(subject.facts["company"].as_i64() == Some(company)
                && bilibi_authz::effective_capabilities_of(sql, scope, account)
                    .await?
                    .allowed
                    .contains(VIEW)
                && self
                    .cover_candidate(sql, scope, company, account, identity)
                    .await?)
        })
    }
    fn maintenance<'a>(
        &'a self,
        sql: &'a Transaction<'_>,
        scope: &'a bilibi_authz::SystemScope,
        subject: &'a Subject,
    ) -> OwnerFuture<'a, ApprovalSubject> {
        Box::pin(async move {
            let company = subject.facts["company"].as_i64().ok_or(Error::Invalid)?;
            let journal = journals::find(sql, scope.tenant(), company, subject.id)
                .await?
                .ok_or(Error::NotFound)?;
            if journal.status != JournalStatus::Submitted {
                return Err(Error::Conflict);
            }
            let book = books::find(sql, scope.tenant(), company)
                .await?
                .ok_or(Error::NotFound)?;
            Ok(approval_facts(journal, book, company))
        })
    }
    fn relationship<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        _: &'a Scope,
        _: &'a Subject,
        _: &'a str,
        _: i64,
    ) -> OwnerFuture<'a, bool> {
        Box::pin(async { Err(Error::Invalid) })
    }
    fn authorize_control<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        _: &'a Scope,
        _: &'a Subject,
        _: &'a str,
        _: &'a str,
    ) -> OwnerFuture<'a, ()> {
        Box::pin(async { Err(Error::Forbidden) })
    }
    fn validate_saved<'a>(
        &'a self,
        _: &'a Transaction<'_>,
        _: &'a Scope,
        _: &'a Subject,
        _: &'a ApprovalIdentities,
    ) -> OwnerFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn apply_in<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        scope: &'a Scope,
        subject: &'a Subject,
        outcome: ApprovalOutcome,
        reason: &'a str,
        _: &'a Value,
    ) -> OwnerFuture<'a, ()> {
        Box::pin(async move {
            let company = subject.facts["company"].as_i64().ok_or(Error::Invalid)?;
            match outcome {
                ApprovalOutcome::Approved => {
                    Ledger::apply_journal_approval_on(tx, scope, company, subject.id)
                        .await
                        .map_err(db_error)?;
                }
                ApprovalOutcome::Rejected => {
                    Ledger::apply_journal_rejection_on(tx, scope, company, subject.id, reason)
                        .await
                        .map_err(db_error)?;
                }
                ApprovalOutcome::Recalled => Ledger::apply_journal_withdrawal_on(
                    tx,
                    scope,
                    company,
                    subject.id,
                    Some(reason.to_owned()),
                )
                .await
                .map_err(db_error)?,
            };
            Ok(())
        })
    }
}

async fn request_id(
    sql: &dyn bilibi_agent_api::OperationClient,
    scope: &Scope,
    company: i64,
    journal: i64,
) -> Result<i64> {
    Ok(sql.query_opt("SELECT request_id FROM finance_ledger_journal_approvals WHERE tenant_id=$1 AND company_id=$2 AND journal_id=$3",&[&scope.tenant(),&company,&journal]).await?.ok_or(LedgerError::ApprovalNotConfigured)?.get(0))
}
async fn link(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    journal: i64,
    request: i64,
) -> Result<()> {
    let tenant = scope.tenant();
    tx.mutate(move |sql|Box::pin(async move {
        let id:i64=sql.query_one("INSERT INTO finance_ledger_journal_approvals(tenant_id,company_id,journal_id,request_id) VALUES($1,$2,$3,$4) RETURNING id",&[&tenant,&company,&journal,&request]).await?.get(0);
        Ok(((),vec![Change {kind:"finance.ledger.journal_approval".into(),id,old:Value::Null,new:json!({"journal_id":journal,"request_id":request})}]))
    })).await?;
    Ok(())
}
pub(crate) async fn submit(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    journal: i64,
) -> Result<()> {
    let request = engine()?
        .submit_approval_in(
            tx,
            scope,
            CASE,
            journal,
            &format!("finance-journal:{journal}"),
            None,
        )
        .await
        .map_err(refusal)?;
    link(tx, scope, company, journal, request).await
}
pub(crate) async fn submission_policy(
    sql: &Transaction<'_>,
    scope: &Scope,
    company: i64,
) -> Result<Value> {
    let (id, policy) = engine()?
        .approval_submission_policy_on(sql, scope, CASE, company)
        .await
        .map_err(refusal)?;
    Ok(json!({"policy_id":id,"rules":policy}))
}
const NO_SEAT: &str = "You cannot decide this journal. The approval rules need another eligible person, such as someone who did not prepare it.";
pub(crate) async fn preview(
    sql: &Transaction<'_>,
    scope: &Scope,
    company: i64,
    journal: i64,
) -> Result<bilibi_workflow::ApprovalItem> {
    let request = request_id(sql, scope, company, journal).await?;
    engine()?
        .approval_detail_on(sql, scope, request)
        .await
        .map_err(|error| match error {
            Error::Forbidden => LedgerError::ApprovalRefused(NO_SEAT.into()),
            error => refusal(error),
        })
}
pub(crate) fn single_step(item: &bilibi_workflow::ApprovalItem) -> Result<()> {
    if item.seats.is_empty() {
        return Err(LedgerError::ApprovalRefused(NO_SEAT.into()));
    }
    if item.seats.len() != 1 || item.request.stages.len() != 1 {
        return Err(LedgerError::ApprovalRefused(
            "More than one approval step or seat is open to you. Open Waiting for my approval and choose the one you are deciding.".into(),
        ));
    }
    Ok(())
}
pub(crate) async fn decide(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    journal: i64,
    approve: bool,
    reason: &str,
) -> Result<Journal> {
    join(tx, scope, company, APPROVE_JOURNALS).await?;
    submitted_journal(tx.sql(), scope.tenant(), company, journal).await?;
    let item = preview(tx.sql(), scope, company, journal).await?;
    single_step(&item)?;
    if approve && item.approval_reason_required && reason.trim().is_empty() {
        return Err(LedgerError::ApprovalRefused("These rules require an approval reason. Decide this journal from Waiting for my approval.".into()));
    }
    let stage = &item.request.stages[item.request.stage_index];
    let mut action = bilibi_workflow::ActionRequest::new(
        ACTION,
        format!(
            "finance-journal-decision:{}:{}:{}",
            item.request.id,
            item.request.version,
            scope.user()
        ),
        item.subject_version,
    );
    action.payload = json!({"request":item.request.id,"version":item.request.version,"stage":stage.key,"seat":item.seats[0],"routing_version":item.request.routing_version,"approve":approve,"reason":reason,"input":{}});
    engine()?
        .execute_action_in(tx, scope, SUBJECT, journal, action)
        .await
        .map_err(refusal)?;
    journals::find(tx.sql(), scope.tenant(), company, journal)
        .await?
        .ok_or(LedgerError::JournalNotFound)
}
pub(crate) async fn recall(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    journal: i64,
    reason: Option<&str>,
) -> Result<()> {
    join(tx, scope, company, SUBMIT_JOURNALS).await?;
    withdrawable_journal(tx.sql(), scope, company, journal).await?;
    let id = request_id(tx.sql(), scope, company, journal).await?;
    let request = engine()?
        .approval_history_on(tx.sql(), scope, id, 0, 1)
        .await
        .map_err(refusal)?
        .request;
    engine()?
        .recall_approval_in(
            tx,
            scope,
            request.id,
            request.version,
            reason.unwrap_or("Journal withdrawn by its submitter"),
        )
        .await
        .map_err(refusal)
}

impl Ledger {
    /// Pure defaults for explicit publication through Workflow. No policy is
    /// installed by boot, opening a book, or submitting a journal.
    pub fn journal_approval_policy() -> ApprovalPolicy {
        policy(&DECLARATION)
    }
}
