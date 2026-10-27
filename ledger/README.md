# Ledger

Module ID: `finance/ledger` (Bilimbi `Bilimbi.Finance.Ledger`). A company's
general ledger: its book, chart of accounts, journal entries, posting
notices, fiscal years with period closing and the year-end close, controlled
bookkeeping and late-recording reviews, reversals, the separation of duties
with its approval exceptions and their reviews, manual journals, the trial
balance and account totals (also by owner) at a pinned entry, and the evidence
datasets of the auditor's pack. Other Finance modules and other Domains post
and read through this module's facade; none of them reads its tables.

## Public API

Resolve the `Ledger` service (`finance/ledger.ledger`) from the registry and
call it with a sealed `Scope` and an explicit company id. Results are plain
records (`Book`, `Account`, `Entry`, `FiscalYear`, `Period`, `ClosingEvent`,
`ControlledBookkeeping`, `LateRecording`, `DutySeparation`,
`ApprovalException`, `Journal`, `TrialBalance`, `AccountTotals`,
`OwnerTotals`, `EvidencePage`, ...); the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `get_book`, `get_book_on` | `finance.ledger.view` | The company's currency code and minor units, or `BookNotOpen`; `_on` uses the caller's connection |
| `open_book` | `finance.ledger.accounts.manage` | Opens the book once with `currency_code` (three letters, trimmed and upcased) and `minor_units` (0 to 4); a second call is `BookAlreadyOpen`. A new company's book is controlled (see Controlled bookkeeping) |
| `list_accounts`, `list_accounts_on` | `finance.ledger.view` | The chart of accounts by code; `_on` uses the caller's connection |
| `create_account` | `finance.ledger.accounts.manage` | Adds an account: `code`, `name`, `account_type`, optional `parent_id`, `active`, `retained_earnings` |
| `update_account` | `finance.ledger.accounts.manage` | Changes `name`, `parent_id`, `active` or `retained_earnings`; `AccountNotFound` for an id of another company |
| `post_entry` | `finance.ledger.entries.post` | Posts a balanced entry of at least two lines (see Entries); `BookNotOpen` before the book is opened |
| `post_entry_in` | `finance.ledger.entries.post` | Same posting inside the owner's `MutationTransaction`; leaves commit/rollback to the owner |
| `preview_post_entry_on`, `preview_reverse_owned_entry_on` | `finance.ledger.entries.post` | Validate an owned posting/correction on the handed `OperationClient`, without identities or writes; deterministic entry, referenced accounts and applicable posting controls for the owner's review fence |
| `write` | caller's supplied capabilities | Opens an audited writable-company transaction, reauthorizing inside it, for a Finance record owner |
| `get_entry` | `finance.ledger.view` | One entry with its lines and the entry that reversed it; `EntryNotFound` for another company's id |
| `list_entries` | `finance.ledger.view` | Entries by date then id, 100 by default and 500 at most, paged with `after`; bounded by `from`, `to`, `account_id` and `through_entry_number`; `exclude_closing` leaves out year-end closing entries and their reversals |
| `list_entries_on` | `finance.ledger.view` | `list_entries` on the caller's connection, for a sibling reading entries inside its own write while it holds the company lock |
| `reverse_entry` | `finance.ledger.entries.post` | Posts a manual entry's reversal (see Reversals) |
| `reverse_owned_entry` | `finance.ledger.entries.post` | The owning module's reversal of its own entry, inside its own transaction |
| `subscribe_postings` | `finance.ledger.view` | A `PostingSubscription` that hears each committed entry of the company (see Posting notices) |
| `open_fiscal_year` | `finance.ledger.periods.manage` | Opens the next fiscal year and its monthly periods, all open (see Fiscal years and periods) |
| `list_fiscal_years`, `get_fiscal_year` | `finance.ledger.view` | Years in date order with their status, closing entry and periods; `FiscalYearNotFound` |
| `list_closing_events` | `finance.ledger.view` | A year's closing log, oldest first |
| `close_period` | `finance.ledger.periods.manage` | Closes an open period, optional `reason` |
| `reopen_period` | `finance.ledger.periods.reopen` | Reopens a closed period with a `reason`; records an approval exception |
| `lock_period` | `finance.ledger.periods.lock` | Locks a closed period for good, optional `reason` |
| `close_fiscal_year` | `.periods.manage` and `.entries.post` | Posts the closing entry and closes the year with its open periods |
| `reopen_fiscal_year` | `.periods.reopen` and `.entries.post` | Reopens the year and its last period, reverses the closing entry; records an approval exception |
| `get_controlled_bookkeeping` | `finance.ledger.view` | The setting, its latest change and the number of fiscal years |
| `set_controlled_bookkeeping` | `finance.ledger.controlled-bookkeeping.manage` | Turns it on, or off with a `reason` (an approval exception); a call that changes nothing records nothing |
| `list_controlled_bookkeeping_changes` | `finance.ledger.view` | Every change, oldest first |
| `list_late_recordings` | `finance.ledger.view` | Entries recorded 45 or more days after their date, with their reviews (see Late recordings) |
| `record_late_recording_review` | `finance.ledger.late-recordings.review` | An independent reviewer's `reason` for one late entry |
| `get_duty_separation` | `finance.ledger.view` | Whether the company requires separation of duties, and its latest change |
| `set_duty_separation` | `finance.ledger.duty-separation.manage` | Turns it on, or off with a `reason`; a call that changes nothing records nothing |
| `list_duty_separation_changes` | `finance.ledger.view` | Every change, oldest first |
| `check_approval` | `finance.ledger.view` | A Finance module's approval, inside its own transaction: refused as `SamePerson` or recorded as an approval exception (see Separation of duties) |
| `list_approval_exceptions`, `get_approval_exception` | `finance.ledger.view` | The exceptions log with each review; `ExceptionNotFound` |
| `review_approval_exception` | `finance.ledger.approval-exceptions.review` | An independent reviewer's reading of one exception, optional `note` |
| `submit_journal` | `finance.ledger.journals.submit` | Keeps a manual journal for approval (see Manual journals) |
| `approve_journal` | `finance.ledger.journals.approve` | Posts a submitted journal as an entry by its approver |
| `reject_journal` | `finance.ledger.journals.approve` | Rejects a submitted journal with a `reason` |
| `get_journal`, `list_journals` | `finance.ledger.view` | Journals with their lines, status and decision; `JournalNotFound` |
| `trial_balance` | `finance.ledger.view` | Every account's totals and one-sided balance as of a date, optionally `through_entry_number` (see Trial balance) |
| `account_totals` | `finance.ledger.view` | Per-account debit and credit totals over entries dated `from` through `to`, optionally `through_entry_number` and `exclude_closing` (see Account totals) |
| `account_totals_by_owner` | `finance.ledger.view` | The same totals, one row per account and owner module, for a control-account tie-out |
| `evidence_page` | `finance.ledger.view` | One ID-ordered page of one of the ledger's tables for the auditor's pack (see Evidence) |
| `Ledger::evidence_datasets` | none | The fifteen dataset names, sorted |
| `Ledger::account_types`, `Ledger::late_recording_thresholds`, `Ledger::journal_statuses` | none | The five account types; the report's 45 and 60 days; `submitted`, `approved`, `rejected` |

Every call authorizes the capability for the scope's actor and the target
company at the moment it runs (Core Company's `authorize_target`): a missing
grant, or a sibling company without `admin.company.tenant-wide.manage`, is
`LedgerError::Unauthorized`; a company the scope cannot see is
`CompanyNotFound`. A write then holds the company's row lock inside an audited
`MutationBoundary` transaction, so an archived company is `CompanyArchived`
and a company's ledger writes run one at a time. Refused values come back as
`LedgerError::Invalid(Vec<FieldError>)` with Bilimbi's changeset messages
(`can't be blank`, `has already been taken`, `must be an equity account`,
`would place the account under itself`, ...).

## Agent operations

H1 makes the facade-only writes reachable through the Platform's shared `/api/v1` and `/mcp` contracts. Discover and describe `finance.ledger.*` and read the `finance.ledger` guide before writing. All inputs name `company_id`; tenant, actor and impersonator come from the sealed credential. Keys, titles and schemas use meta-terms; company-specific discovery wording belongs in `keywords` (none are hardcoded here).

| Suffix under `finance.ledger.` | Contract |
| --- | --- |
| `book.get` | Book or null, controlled-bookkeeping and duty-separation changes |
| `account.list` | Database-code-ordered chart; `after_code`, `next_after_code` |
| `journal.get`, `journal.list` | Lines and decision; list by number with optional `status`, `after`, `next_after` |
| `entry.get`, `entry.list` | Immutable lines and ownership/reversal identifiers; list filters `from`, `to`, `account_id`, `through_entry_number`, `exclude_closing`; cursor pair `after_date`, `after_id` in `next_after` |
| `fiscal_year.list` | Years with periods/statuses; `after_start_date`, `next_after_start_date` |
| `trial_balance.get` | Every account with a posting on or before `as_of`, with debit and credit balances, counted through a pinned `through_entry_number` |
| `book.open`, `account.create`, `account.update` | Reviewed book/chart changes; account update preserves omitted fields and uses `clear_parent` to remove a grouping |
| `journal.submit`, `journal.approve`, `journal.reject` | Submission does not post; approval posts once with separation of duties; rejection records a reason without posting |
| `entry.reverse` | Mirror a manual entry once, never an owned or closing entry |
| `fiscal_year.open`, `fiscal_year.close`, `fiscal_year.reopen`, `fiscal_year.lock` | Open monthly periods, close with computed entry, reopen with closing reversal, or atomically lock all remaining closed periods |
| `period.close`, `period.reopen`, `period.lock` | Existing facade ordering, reason and permanent-lock rules |

The eight reads return at most `page_size` records (1–100, default 100) on list operations. The fourteen writes require shared plan preview/commit; direct calls refuse writes. Preview validates normalized/defaulted values on the caller's connection, without business mutations, and shows exact proposed entry lines, current accounts/periods and consequential designation changes. Amounts are decimal strings, never binary floats. Commit reauthorizes and joins the shared audited transaction through private facade methods; all operations and audit rows roll back together on refusal. A successful record response has `path: null`: a write commits no page path.

The pinned Platform previews every step before executing any. A transaction-local FIFO of review hashes rechecks each step immediately before mutation, so earlier steps cannot silently change later reviewed facts. Dependent steps need separate plans. Same-company account edits conservatively review the full chart, so even independent account edits need separate plans. Whole-year lock composes existing period locks in date order on the same transaction; it adds no year-lock event. No operation exposes direct posting, owner/settlement bypasses or disabling controls. See [guide.md](guide.md) for consent, staleness and safe ordering.

### Live HTTP and MCP proof

Use a new disposable PostgreSQL 18 database and run these commands from the mounted Platform root. The bootstrap installs only fixture records, issues real device/API and resource-bound PKCE/MCP credentials for two independent people and another tenant, and saves them with mode 0600 using `create_new`. Do not use a production database or print the credential file. The existing facade walkthrough remains the default binary.

```sh
export DATABASE_URL=postgres://…/empty_agent_proof
export BILIBI_AGENT_PROOF=disposable
export BILIBI_AGENT_CREDENTIALS=/tmp/ledger-agent-credentials.json
export APP_ORIGIN=http://localhost:3100
cargo run -p bilibi-finance-walkthrough-fixture --bin agent-operations
cargo build -p bilibi-web
amp orb service start ledger-proof --port 3100 --command \
  'BIND_ADDR=127.0.0.1:3100 ./target/debug/bilibi-web run'
python3 apps/domains/finance/fixtures/walkthrough/agent-operations.py
```

The wire walkthrough exercises all writes over both transports, observes preview non-mutation and code non-consumption on rollback, checks exact approval/reversal/closing amounts, traverses mixed-case/non-ASCII account cursors, and refuses precision, balance, same-person approval, closed/locked periods, double reversal, cross-tenant reads and actor spoofing. Two defaulted year opens in one plan prove the intra-plan fence and rollback of the first insert. Credentials expire after one hour; rerun on another fresh database for another proof.

## Pages

The ledger has pages so a person without an AI agent can do the same daily work. Bilimbi's ledger has none (`web: nil`), so these are this port's own. Reports keeps the older [Bookkeeping and Approval exceptions pages](../reports/README.md#bookkeeping-and-approval-exceptions-pages).

| Menu (Finance, under Bookkeeping) | Route | What a person does there |
| --- | --- | --- |
| Book | `/finance/ledger/book` | Reads the currency and decimal places. Opens the book once, asking who approves. Sees whether controlled bookkeeping and separation of duties are on. |
| Chart of accounts | `/finance/ledger/accounts` | Searches and filters the chart, adds an account in a dialog. |
| (account record) | `/finance/ledger/accounts/{id}` | Edits the name and group in place, deactivates or activates the account, marks the retained earnings account, reads today's balance. |
| Journal entries | `/finance/ledger/entries` | Lists entries oldest first, 100 at a time. The shared period filter narrows by entry date; with no period, every date is listed. |
| (entry record) | `/finance/ledger/entries/{id}` | Reads the lines. Reverses a manual entry in a dialog. |
| Manual journals | `/finance/ledger/journals` | Lists journals by status. Submits one with its lines in a dialog. |
| (journal record) | `/finance/ledger/journals/{id}` | Approves it (it posts) or rejects it with a reason. The ledger refuses the person who submitted it while the company separates duties. |
| Fiscal years | `/finance/ledger/fiscal-years` | Opens a year after a review that names the company and shows the calculated last day. Closes, reopens and locks periods. Closes, reopens and locks a year. |
| Trial balance | `/finance/ledger/trial-balance` | Reads the balances as of a date at a pinned entry number, and follows an account to its entries. The as-of day is one date, not a range, so it keeps its own date control rather than the period filter. |

Every read goes through `PageContracts::read` and every write through its `submit` (a dialog the person just filled in) or `preview` then `commit` (a confirmation), so a page and an agent get the same checks, the same refusals and the same audit trail. The pages add one contract, `finance.ledger.trial_balance.get`, because the trial balance had none. A control the person may not use is not drawn. The ledger's own words explain a refusal.

Two limits come from the contracts. The entry and journal lists are oldest first and show a "Later entries" link at 100 rows, because the contracts page by a cursor. The chart loads whole (up to 5,000 accounts) and the page searches and sorts it.

Journal entries use the shared period filter (`bilibi_datetime::PeriodFilter`, `apps/base/datetime/README.md`) beside the account toolbar. Entry dates are civil dates, both inclusive. A preset follows the display clock, including the browser's zone when the display mode is local, and is read again on each visit: an address kept overnight with `period=today` shows the new day and starts at its first entry. Until the browser has reported its zone in local mode, the address's own days stand, so a reload or Back keeps its place in the list. The page does not turn those days into UTC instants. Applying a period writes `period`, `from` and `to`, and starts again at the first matching entry; the account filter and the trial balance pin `through` stay. A `to` with no `from` and no `period` means on or before that day, which is how the trial balance opens this list. An address with no date keys lists every entry, not the shared default of the last 30 days, because the trial balance and the account pages open this list by account alone. The control then shows a custom range of 0001-01-01 through 9999-12-31, and an open end is not sent to the ledger.

Run the proof on a disposable PostgreSQL 18 database with `DATABASE_URL=… APP_ORIGIN=http://127.0.0.1:4361 BIND_ADDR=127.0.0.1:4361 BILIBI_SETTINGS_KEY=$(openssl rand -hex 32) cargo run -p bilibi-finance-walkthrough-fixture --bin ledger-ui`. It seeds a chart, two fiscal years with closed periods, entries, and manual journals in each state, then serves the app. Sign in as `manager@other.example.test` (submits), `reviewer@other.example.test` (approves) or `viewer@other.example.test` (reads), password `bilibi-dev`. Add `-- empty` to start with no book. [`screenshots/`](screenshots) shows the pages at desktop and phone width.

## Capabilities

Twelve keys, verbatim from Bilimbi, under the `finance` domain, all granted to
`tenant_owner`: `finance.ledger.view`, `.accounts.manage`, `.entries.post`,
`.periods.manage`, `.periods.reopen`, `.periods.lock`,
`.duty-separation.manage`, `.controlled-bookkeeping.manage`,
`.late-recordings.review`, `.journals.submit`, `.journals.approve` and
`.approval-exceptions.review`. The module declares the verbs `post`, `reopen`
and `lock` as Bilimbi does, and also `submit` and `review`, which no Bilibi
Platform module declares (Bilibi merges verbs across modules).

## Money

`Money` is a `rust_decimal` amount held to a book's `MinorUnits`:
`Money::parse("12.50", units)` and `Money::new(decimal, units)` refuse an
amount with more decimal places than the book's minor units (`has more than N
decimal places`) and one of sixteen or more digits before the point, the
`numeric(20,4)` column's range. `shown(units)` writes it with exactly the
book's decimal places.

## Entries

`post_entry(scope, company_id, &NewEntry, &PostOptions)` posts one entry of
`entry_date`, `description` (1 to 500 characters), an optional `reference` (at
most 100) and `lines`: `NewLine::debit(account_id, money)` or
`NewLine::credit(..)`, each with an optional description. It is refused as
`LedgerError::Invalid(Vec<FieldError>)`, with every refusal at once, unless
there are at least two lines (`needs at least two lines`), debits equal
credits (`debits and credits do not balance`), each line has exactly one
positive side (`a line has either a debit or a credit`) no finer than the
book's minor units (`has more than N decimal places`), and each line names an
active account of the company (`is not an account of this company`,
`is inactive`). A `FieldError` about a line carries its 1-based `line`.

When a Finance owner must record its own posting in the same commit, open `Ledger::write(client, scope, company_id, capabilities, action)` and call `post_entry_in(&mut tx, scope, company_id, new, options)`. Both its capability and Ledger's are checked; the owner returns every changed row as a `Change` and owns commit/rollback. Never call standalone `post_entry` while holding the company lock on another connection: it would wait on itself. `reverse_owned_entry` already joins that same owner transaction.

Agent preview seams accept `OperationClient`, as do `get_book_on` and `list_accounts_on`, so a caller never obtains another connection while holding the company lock. Posting/correction preview requires posting authority, not view, and reuses the actual posting/owner/reversal validation. It takes the writable company lock without waiting, returns deterministic review facts and allocates no identity or number. Only the posting date's period and referenced accounts are frozen, not unrelated fiscal years or chart rows. If the record has no ID yet, preview without an owner and state its symbolic ownership in the caller's review; execution must reserve its ID and supply the real owner to `post_entry_in`. Preview is not a posting capability or reservation; execution still reauthorizes and validates.

`PostOptions` carries Bilimbi's two options. `settlement_account_ids` lists
accounts that may be inactive in this entry only, for a module settling a
document it posted earlier; each must still be an account of the company.
`owner` is the module (its descriptor id, `finance/receivables`) and record
(`<kind>:<id>`, `payment:17`) that recognise the entry. A record has at most
one original entry per company (`owner_record already has an entry`), and an
owned entry is corrected only by its owner. An `owner`, or an id, of another
shape is `InvalidOption`. A manual entry has no owner.

The database numbers each company's entries 1, 2, 3, ... without gaps and
stamps `posted_at`, whatever the writer sent. `Entry` is read back from what
PostgreSQL stored. Reading an entry authorizes `finance.ledger.view` afresh,
and an entry id of another company is `EntryNotFound`.

An entry's date must fall in an open period: never in a closed or locked one
(`entry_date falls in a closed period`), never before the company's first
fiscal year (`is before the first fiscal year`), and, while the company is
controlled, never where no period exists (`falls in no fiscal period: open the
fiscal year that covers it first`). The database refuses the same dates on its
own, and a posting holds a share lock on its period until it commits, so a
concurrent close waits for it.

## Reversals

A posted entry is never edited or deleted. `reverse_entry(scope, company_id,
entry_id, &Reversal)` posts its mirror image (each debit a credit and the other
way round, with the lines' descriptions) on `entry_date`, not before the
original's (`entry_date cannot be before the reversed entry's date`), with its
own `description` and optional `reference`. An entry is reversed once
(`AlreadyReversed`), a reversal is never reversed (`ReversalNotReversible`:
post a new entry), and a year-end closing entry is reversed only by
`reopen_fiscal_year` (`ClosingEntry`). Accounts deactivated since the original
still take its reversal, but its date must fall in an open period, as a new
entry's does. An owned entry is `OwnedBy(owner)`: correct it through its
owner.

`reverse_owned_entry(tx, scope, company_id, entry_id, &owner, &Reversal)` is
the owning module's correction path. The owner calls it on the
`MutationTransaction` that records its own reversal record (a payment
reversal, a cancellation); the ledger authorizes and takes the company lock on
that transaction and leaves the commit to the owner, so both stand or
neither. The reversal carries the same owner. Another module's or record's
entry is `NotTheOwner`, a manual entry `NotOwned`, and a malformed owner
`InvalidOption`. The owner is a seam, not an authorization boundary: the
ledger cannot tell which module calls, so anyone holding
`finance.ledger.entries.post` who passes an entry's owner reverses it without
the owner's record, and the owner's tie-out shows the difference.

## Separation of duties

A company requires separation of duties unless it turns it off. The setting
is the latest of its append-only changes (`DutySeparationChange`); with none,
separation is required. `set_duty_separation(scope, company_id, required,
reason)` turns it off only with a reason (`reason is required to turn
separation off`).

`check_approval(tx, scope, company_id, &Approval)` is how every Finance
approval asks, on the approving module's own `MutationTransaction` (it
authorizes `finance.ledger.view` and takes the company lock there, so a
refusal rolls the approval back). `Approval` names the `source` module, an
`action` and `subject_type` code, the `subject_id`, a `subject` label for the
reviewer, everyone in `prepared_by` (`Preparer { user_id, impersonator_id }`)
and, for an approval that overrides a control, its `override_reason`. Two
people overlap when either one's user or impersonator is the other's user or
impersonator, so impersonation counts both ways. A preparer who overlaps the
approver is `SamePerson` while separation is required; while it is off, the
approval is allowed and recorded as a same-person approval exception with the
reason separation was turned off, naming that change. An override is always
recorded with its own reason. Anything else records nothing. The ledger's own
reopens and turning controlled bookkeeping off are recorded as overrides
(`period_reopen`, `fiscal_year_reopen`, `controlled_bookkeeping_off`, source
`finance/ledger`, no preparer).

`list_approval_exceptions` lists the exceptions oldest first, each with its
review; `ApprovalExceptionQuery` bounds `recorded_from` (inclusive) and
`recorded_before` (exclusive) instants, filters `reviewed`, and pages by
`after` (the last exception number) and `limit` (1 to 500, 100 by default;
`InvalidOption` outside). `review_approval_exception(.., note)` records one
independent review (`AlreadyReviewed` after), never by whoever approved or
prepared what it records or anyone overlapping them
(`ReviewerNotIndependent`), whatever the setting; a note is at most 500
characters.

## Manual journals

`submit_journal(scope, company_id, &NewEntry)` keeps a journal for approval:
the same fields and refusals as `post_entry` (balanced, active accounts of the
company, an open period), numbered per company and stamped by the database,
with the submitter and any impersonator. Nothing posts until
`approve_journal`, which posts it as an entry on its date, by the approver,
and records the `approved` decision naming the entry. The entry is checked
again when it is approved: a period closed or an account deactivated since is
`Invalid`, and the journal stays submitted. The approver must not overlap the
submitter (`SamePerson`) unless separation is off, when the approval is
recorded as a same-person exception (`journal_approval`, subject `Journal
<number>: <description>`). `reject_journal(.., reason)` needs a reason
(`reason is required to reject a journal`). Anything but a submitted journal
is `JournalDecided`. `list_journals` filters by `status` and pages like the
exceptions.

## Posting notices

`subscribe_postings` returns a `PostingSubscription` once its listening
connection is up, so it hears every entry committed afterwards.
`subscription.next().await` yields `PostingEvent::Posted { company_id,
entry_number }` for each committed entry of the subscribed company, from any
process, and `PostingEvent::Refresh` when notices may have been missed (the
connection dropped, or the subscriber lagged): read the ledger again. The
`finance_ledger_entries_notify` trigger calls `pg_notify` in the posting
transaction, and PostgreSQL delivers it only at commit, so a refused or
rolled-back post announces nothing. A notice carries identifiers only; read
figures through `get_entry`/`list_entries`. The process holds one dedicated
connection, opened by the first subscription from `DATABASE_URL` (the variable
the Platform builds its pool from) and re-opened every five seconds after a
failure; dropping a subscription ends it.

## Fiscal years and periods

`open_fiscal_year(scope, company_id, &NewFiscalYear)` opens the company's next
year, divided into calendar-month periods. `start_date` is the first day of a
month (`must be the first day of a month`) and `end_date` the last day of a
month (`must be the last day of a month`), one to 24 months on (`must end one
to 24 months after the start`); the end defaults to twelve months on. The
first year may start on any month; each later year starts the day after the
previous one ends (`must be <date>, the day after the previous fiscal year
ends`), which is also its default. `BookNotOpen` before the book is opened.

A status is never stored: it is the latest closing event's (open with none).
`close_period` closes in date order (`EarlierPeriodOpen`; `PeriodNotOpen` for a
closed or locked period), and a year's last period closes only with its year
(`ClosesWithFiscalYear`). `reopen_period` needs a reason (`reason is required
to reopen`) and reopens in reverse date order (`LaterPeriodClosed`); a locked
period never reopens (`PeriodLocked`), an open one is `PeriodNotClosed`, and a
closed year's periods reopen only after the year (`FiscalYearClosed`).
`lock_period` locks a closed period in date order (`EarlierPeriodUnlocked`).
Reasons are trimmed and at most 500 characters. Each call returns the period
with its new status.

`close_fiscal_year(scope, company_id, year_id, &YearClose)` posts one entry,
dated the year's last day (description `Close fiscal year <start> to <end>`
unless given, optional reference), that brings each income and expense
account's balance for the year to zero against the active retained-earnings
account (`NoRetainedEarningsAccount` without one), then records the year's
close (naming the entry and the account) and closes every open period of the
year. A year with no income or expense balance closes with no entry; the
previous year's periods must be closed first (`EarlierPeriodOpen`); closing a
closed year returns it unchanged. `reopen_fiscal_year(.., reason)` reopens
the year and its last period and posts the closing entry's reversal on the
same day (`Reopen fiscal year <start> to <end>`); the other periods stay
closed. It is `FiscalYearNotClosed`, `PeriodLocked` when the last period is
locked, and `LaterPeriodClosed` while the next year has a closed period.
Closing entries and their reversals may name inactive accounts, are never
reversed by hand, and are left out of `list_entries` with `exclude_closing`
and of the late-recording report.

A reopen overrides a close and turning controlled bookkeeping off overrides a
control, so each is recorded in `finance_ledger_approval_exceptions` with its
reason (`period_reopen`, `fiscal_year_reopen`, `controlled_bookkeeping_off`,
source `finance/ledger`); see Separation of duties.

## Controlled bookkeeping

A company that is controlled posts only into a fiscal period it has opened
and left open, so its first year is opened explicitly and an entry dated after
its last year is refused until the next one is opened; an entry is never
refused for being old. The setting is the latest of the company's append-only
changes (`BookkeepingChange`: number, setting, reason, actor, impersonator,
time); `open_book` records the change that turns it on, so a new company is
controlled from the start. `set_controlled_bookkeeping(scope, company_id,
controlled, reason)` turns it off only with a reason (`reason is required to
turn this off`) and returns the new `ControlledBookkeeping`; setting what is
already set records nothing.

## Late recordings

An entry's days are whole UTC days from its `entry_date` (its transaction
date) to `posted_at` (its recording). `list_late_recordings` lists the entries
of 45 days or more, oldest number first, `Approaching` up to 60 days and `Late`
past it, each with its review; `LateRecordingQuery` filters by `status` and
`reviewed` and pages by `after` (the last entry number) and `limit` (1 to 500,
100 by default; `InvalidOption` outside). `record_late_recording_review` is
refused for an entry that is not listed (`NotLateRecorded`; `EntryNotFound`
for another company's or a closing entry), a second time (`AlreadyReviewed`),
and for a reviewer who posted the entry or impersonated (or was impersonated
by) its poster (`ReviewerNotIndependent`); its `reason` is required.

## Trial balance

`trial_balance(scope, company_id, as_of, through_entry_number)` lists every
account with a posting dated on or before `as_of`, by code, with its `debits`
and `credits` and its balance on one side (`debit_balance` or
`credit_balance`, the other zero). Each account reports its own postings
only; `parent_id` lets a reader group rows without counting a parent twice.
`total_debit` equals `total_credit` because every entry balances. Closing
entries count, so as of a closed year's last day every income and expense
account is at zero and the year's result is in retained earnings.

The result names `through_entry_number`, the last entry it counts: the one
passed, or the company's last, read in the same statement as the sums. An
entry posted later with an earlier date changes the figures for that date, so
passing the same `as_of` and `through_entry_number` reproduces an earlier
trial balance's figures exactly: entries are numbered in posting order and
never change. Account names, parents and active flags are read as they are
now. A negative number is `InvalidOption`, and `BookNotOpen` comes before the
book is opened.

## Account totals

`account_totals(scope, company_id, &TotalsQuery)` returns, for every account
with a posting dated from `from` through `to` (both optional and inclusive;
`from` after `to` is `InvalidOption`), its `debits` and `credits`, by code.
PostgreSQL sums the lines per account, so a report over any number of
entries reads one row per account; never add up `list_entries` lines for a
figure. It names `through_entry_number` like the trial balance, and reading
several windows at one number gives a report's columns one state of the
ledger. `exclude_closing` leaves out every year-end closing entry and every
reversal of one, so income and expense over a closed year show its activity
(a profit and loss reads this way, a balance sheet counts them); the set is
the same at any entry number, because a close names its entry in the
transaction that posts it. `list_entries` with the same `account_id`,
`from`, `to` and `through_entry_number` lists the entries behind a figure.

`account_totals_by_owner` splits the same totals by `owner_module` (the
owner's descriptor id, `None` for entries with no owner, first): one row per
account and owner. A module ties its control account out with it: its own
rows should be the whole account, and any other row is a posting its records
do not know, shown as a difference with its owner, never netted away.
Bilimbi's `legacy` flag is not ported, since Bilibi has no adopted entries.

## Evidence

`evidence_page(scope, company_id, dataset, limit, after)` reads one
ID-ordered page of one dataset with its explicit `fields`, its `rows` (JSON
objects) and `next`, the last id when the page was full; `limit` is 1 to 500
(500 when absent) and `after` the previous page's `next`. An unknown dataset
or an option out of range is `InvalidOption`. The datasets are `account`,
`approval_exception`, `approval_exception_review`, `book`,
`bookkeeping_change`, `closing_event`, `duty_separation_change`, `entry`,
`fiscal_year`, `journal`, `journal_event`, `journal_line`,
`late_recording_review`, `line` and `period`, with Bilimbi's field lists
(`src/evidence.rs`), less the entry's `legacy`. Dates are `YYYY-MM-DD`,
instants ISO 8601 UTC with microseconds and amounts the column's exact text
(`100.0000`). Each page is a live read on its own connection. The field lists
are an export contract: freeze one once exported and never add a credential.

## Schema (versions 001 to 005)

`finance_ledger_books` (one per company; currency and minor units fixed for
good), `finance_ledger_accounts` (code and type kept by a trigger, a
composite `(id, company_id)` parent key so an account is never grouped under
another company's, one retained-earnings account per company, never deleted)
and `finance_ledger_bookkeeping_changes` (append-only, numbered and stamped by
the database, refusing a change that changes nothing). Books and bookkeeping
changes refuse UPDATE, DELETE and TRUNCATE.

`finance_ledger_entries` and `finance_ledger_entry_lines` (002) are
append-only the same way. Composite `(id, company_id)` keys keep a line on its
own company's entry and accounts and a reversal on its own company's entry;
the entry's `(currency_code, company_id)` names its book. Triggers number
entries per company without gaps, refuse a line added to an entry whose
transaction has ended, and, deferred to commit, refuse an entry with no lines
or whose debits and credits differ. A line has exactly one positive side. An
owned record has one original entry, and a reversal carries exactly its
original's owner. 003 adds the notice trigger.

004 adds `finance_ledger_fiscal_years`, `finance_ledger_periods` and
`finance_ledger_closing_events` (all append-only): a year starts the day after
the previous one and, deferred to commit, is divided into whole months through
its end, in the transaction that opens it; the database numbers and stamps
each event and refuses one that breaks the closing rules above, a reopen with
no reason, and a closed year with an open period. An entry dated in a closed
or locked period, before the first year, or (controlled) in no period is
refused. `finance_ledger_late_recording_reviews` takes one review per entry,
refusing an entry recorded under 45 days late and a reviewer who overlaps the
poster. `finance_ledger_duty_separation_changes`,
`finance_ledger_approval_exceptions` and
`finance_ledger_approval_exception_reviews` are the end state of Bilimbi's
separation-of-duties tables, all append-only: the database numbers and stamps
each row, refuses a change that changes nothing, a same-person exception
unless the change it names is the latest and turns separation off, and a
review by anyone overlapping the approver or preparer.

005 adds `finance_ledger_journals`, `finance_ledger_journal_lines` and
`finance_ledger_journal_events` (all append-only): the database numbers and
stamps a journal, refuses a line added after the submitting transaction and,
deferred to commit, a journal with no lines or out of balance; a journal is
decided once, an approval names its entry and a rejection its reason, and an
approval by its submitter (or anyone overlapping them) is refused unless the
same transaction recorded its same-person exception.

## Standards addressed

A book is opened once with its currency fixed; an account keeps its code and
type because entries are posted against them, and is deactivated only once
its balance is zero (the sum of its lines); a closed period takes no entry
until it is reopened with a reason that an independent reviewer can read, and
a locked one never reopens; a late entry is reviewed by someone other than
its poster; a posted entry is corrected only by a reversal, once, and an owned
one only through its owner; the person who prepares a record does not
approve it unless the company has turned separation off, and every such
approval and every override is listed for an independent reviewer; a manual
journal posts only once it is approved, by someone independent while
separation of duties is required; any past trial balance or
account total reproduces exactly from its `through_entry_number`; every
table is exported to the auditor field by field; every write is attributed
to the signed-in actor and any impersonator through the Platform's audit
capture.

`evidence_page_on` is the same authorized evidence contract on a caller-supplied PostgreSQL client or transaction. Use it for every page of one repeatable-read export (Reports auditor pack); the pooled `evidence_page` wrapper is a live read per page. Schema helpers remain private.

Opening a book through `finance.ledger.book.open` may include `separation_required: false` and `approval_reason` for a one-person company. That choice also requires `finance.ledger.duty-separation.manage`, records the duty-separation change in the same transaction and keeps every later same-person approval in Approval exceptions. Omitting it leaves the approval control unchanged; independent approval is the default. The Book page asks who approves before opening. A same-person refusal names the control and the Approval exceptions address.

`Money::display` formats a page amount with thousands separators and the book's decimal places. `Money::shown` remains plain decimal notation for editable values.
