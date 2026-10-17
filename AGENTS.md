# bilibi-dom-finance Agent Guide

This repository is the Finance Domain for Bilibi. It is not a standalone Cargo
workspace: it builds only when mounted in a Bilibi checkout at
`apps/domains/finance` (see `README.md`). Read Bilibi's `AGENTS.md`,
`apps/domains/AGENTS.md`, `apps/base/ui/AGENTS.md` and
`apps/base/ports/README.md` in that checkout before changing anything here;
they govern this code and are not copied here.

## Shape of the Domain

- Keep each module deep behind one small facade (`ledger/src/lib.rs`'s
  `Ledger` and its record types), and keep its schema, SQL, validation and
  helpers private (`mod`, `pub(crate)`), not `pub`: a sibling that reaches a
  table or helper couples itself to this module's storage, which the facade
  exists to hide. Add a `pub` only for something the facade's contract names.
- Never add a Cargo dependency, descriptor edge, port call or SQL reference to
  another Domain's container (Supply Chain, Factory, People, ...): cross-domain
  flows will use Base domain events, a Platform slice in progress. Until it
  lands, leave a clearly named `TODO(domain events)` seam at the place the
  flow would start, and do not reach across. Bilimbi's `OrderSource` and
  `PeopleSource` seams are not ported as ports.
- Within this Domain, depend on a sibling module only through its facade,
  declared both in the descriptor's `dependencies` and the Cargo manifest,
  only where Bilimbi has the same edge (`finance/attachments` ← `ledger`,
  `receivables` ← `attachments`, ..., `reports` ← all), and never in a cycle:
  the registry refuses an undeclared service reference and a cycle, and an
  edge Bilimbi lacks is new coupling the parity port must not add.
- Mount a real copy (`git clone`), not a symlink, and commit only inside this
  repository: `apps/domains/Cargo.toml`, `apps/domains/src/lib.rs` and
  `Cargo.lock` belong to the Platform in their unmounted form.
- Use meta-terms only (book, account, account type, entry, line, journal,
  period, duty separation, approval exception, ...). A chart of accounts or a
  currency is a company's data, never a code identifier, schema constant or
  default; the five account types are the one fixed classification.
- Name a migration `finance-<module>-NNN-<slug>` with the SQL file under the
  module's `migrations/`, declared in the descriptor; the registry refuses any
  other name. An applied file never changes (the migration ledger refuses a
  changed hash): a schema change is the next version.

## Facade rules

- Authorize every facade call inside the facade with
  `bilibi_company::organisation::authorize_target` and the scope it was
  handed, and open every write with `Ledger::write` (the capability, then the
  audited `MutationBoundary` transaction holding
  `organisation::require_writable(.., lock = true)`), never a bare
  `client.transaction()`: the boundary is what records every affected row as a
  `Change`, and the company lock is what serializes a company's ledger writes
  and refuses an archived company. Never take an actor or actor id as an
  argument.
- Decide a refusal (a taken code, a cycle, a balance) with `tx.sql()` before
  `tx.mutate`, and report it as a `LedgerError` variant or `FieldError`s with
  Bilimbi's words, never as `Error::Forbidden`/`Error::Internal`: a mutate
  closure that returns no `Change` poisons the transaction, and the live
  transport treats those two errors as fatal.
- Hold amounts as `Money` built with the book's `MinorUnits` (`Money::parse`,
  `Money::new`), never a bare `Decimal` from input: the columns are
  `numeric(20,4)`, and an amount finer than the book's minor units is refused
  by the type, not rounded by PostgreSQL.
- Prove behaviour with `fixtures/walkthrough` on a disposable database
  (`cargo run -p bilibi-finance-walkthrough-fixture`), extending its chart and
  scenarios, not with unit tests or mocks: the gate is
  `scripts/domain-ci.sh` mounted, and the running proof is the facade's own
  answers.
- The ledger owns no CSS, JavaScript, pages or menu items (`web: nil` in
  Bilimbi); a Finance module with pages uses only `bilibi_ui` components and
  `FlexTable` for every list, per `apps/base/ui/AGENTS.md`.
- Agent writes join the shared plan's audited transaction through the ledger's private `*_on` methods, never the pool-owning public wrappers: nested commits would escape plan rollback. Preview on the handed `OperationClient`; retain the transaction-local review fence because the pinned engine previews every step before executing any.

- Post an entry only through `Ledger::post_entry` and read it through
  `get_entry`/`list_entries`; never insert into or select from
  `finance_ledger_entries`/`_entry_lines` from a sibling. Pass `owner` for an
  entry a module's record recognises (the database allows one original entry
  per record and refuses a reversal by anyone but the owner), and
  `settlement_account_ids` only to settle a document posted earlier on an
  account deactivated since. The database numbers entries, so never send or
  compute an `entry_number`.
- Use `Ledger::post_entry_in` inside `Ledger::write` when your record must
  commit with its entry; never call standalone `post_entry` while holding
  that company's lock on another connection, which would wait on itself.
- A new book is controlled, so it posts only into a fiscal period opened with
  `Ledger::open_fiscal_year`: a fixture opens one before posting, and a sibling
  reports the ledger's `entry_date` refusal instead of picking another date. A
  period's or year's status is its latest closing event's, never a column;
  inside the ledger read it through `periods::state` and record each override
  of a control (a reopen, controlled bookkeeping off) with
  `duties::record_override` in the same write.
- Ask `Ledger::check_approval` on your own `MutationTransaction` before every
  approval you record, with everyone whose work it approves in `prepared_by`
  and an `override_reason` for an override; never compare actor ids yourself:
  the ledger counts impersonation both ways, records the exception an
  independent reviewer reads, and a `SamePerson` refusal must roll your
  approval back. Correct an entry you own with `Ledger::reverse_owned_entry` on
  the transaction that records your own reversal, never `reverse_entry`, which
  refuses it as `OwnedBy`.
- Learn of a posting through `Ledger::subscribe_postings`, never your own
  LISTEN: the relay shares one connection, and a notice carries identifiers
  only. On `PostingEvent::Refresh` read again instead of trusting the stream.
  `TODO(platform)`: the relay opens its connection from `DATABASE_URL` because
  `base/database` exposes only the pool.
- Read a ledger figure with `Ledger::account_totals` or `trial_balance`
  (PostgreSQL sums the lines), never by adding up `list_entries` lines, and
  read every column of one report at the first read's `through_entry_number`
  so it reproduces. Tie a control account out with
  `account_totals_by_owner`: a row of another owner is a difference to show,
  never to net away.
- Export a module's auditor evidence as the ledger does
  (`ledger/src/evidence.rs`): `evidence_datasets` and an authorized
  `evidence_page` on the facade, one dataset per own table with a frozen,
  explicit field list (never a credential), ID-ordered pages of 1 to 500.
  The auditor pack reads each module through its facade, never its tables.

- Keep a record's source documents through `Attachments` (`attach`, or
  `capture_file` then `attach_capture` inside your own transaction), never
  through `bilibi_artifacts` or the `finance_attachments*` tables: the facade
  is what links the bytes to the record, applies the company's retention years,
  numbers and audits the attachment, and records holds. Describe your record
  with a `Source` contributed to `finance/attachments.sources` under
  `<your module id>/<kind>` that reads it on the connection it is handed; do
  not hand the attachments module a facade of yours, which would make a cycle.
  Call `refresh_retention` in the transaction that finalises a record whose
  date can change while it is a draft; never extend retention yourself.
- Name a system principal as one lowercase hyphenated segment
  (`finance-attachments-retention-refresh`): Bilibi's vocabulary refuses
  Bilimbi's dotted names. Declaring one grants nothing; a tenant administrator
  grants it (`Grants::set_system`) or its jobs cancel as `actor_refused`.
- Read a company's e-invoice configuration only through `Einvoice::setup`, and
  its code lists only through `list_codes`/`override_code`; never select
  `base_settings` rows or `finance_einvoice_*` tables from a sibling. A secret
  (client secret, certificate, private key) is never returned by the facade: a
  later slice that signs or submits reads it inside `finance/einvoice` only,
  masked everywhere else.
- Put what one clearance authority defines (document types, posting rules,
  shapes, versions, windows, URLs) behind `Authority` in `myinvois.rs`, not in
  the facade, a setting or a page; a second authority is a second
  implementation, not a branch.
- Correct a fiscal document with a new status event or a note, never an edit:
  the e-invoice tables except `finance_einvoice_code_overrides` are
  append-only in the database, and a schema change is the next migration.
- Declare a setting with the `define!` macros in `configuration.rs`, with the
  editor `capability` set; a validator must accept `null` for a nullable value,
  because Base Settings runs it on cleared values too.
- Read sibling context inside a write with `Ledger::get_book_on`,
  `list_accounts_on`, `get_entry_on` and `Payables::get_document_on`, not pooled
  methods: another connection while holding the company lock can exhaust the pool.
- Register assets through `FixedAssets`, never by posting or claiming their
  acquisition entry: it freezes one whole source debit and its revision;
  projected schedules are not posted depreciation (see `fixed_assets/README.md`).
- Change receivable drafts through `Receivables`, not its tables: null/missing
  lines preserve snapshots, an array replaces them atomically, and discard
  keeps them. Issue through `issue_document` and settle through `record_payment`
  / `reverse_payment`: they retain each invoice's original control after setup
  changes and commit its record with the owned entry. Read dated figures with
  the report facade at one entry pin, never the unbounded document outstanding.
- Validate a document to issue with `Draft::validate` (the shape every
  authority shares, then `Authority::validate`), never with checks of your own
  on the facade: it derives every amount and answers `FieldError`s at the
  field's path in Bilimbi's words. Write the authority's JSON only through
  `myinvois::json::Json` and build and sign it only through `myinvois::ubl`
  and `myinvois::signer` (or `Authority::payload`), never `serde_json`: the
  authority hashes and signs exact bytes, and `serde_json`'s maps sort keys
  and reformat numbers. Read a certificate's facts with `Certificate::parse`
  and report them, never the PEM.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this project.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
When updating this file, preserve this bar for all agents and keep entries concise.
