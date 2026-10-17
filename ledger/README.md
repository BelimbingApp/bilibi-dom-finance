# Ledger

Module ID: `finance/ledger` (Bilimbi `Bilimbi.Finance.Ledger`). A company's
general ledger: its book, chart of accounts, journal entries, posting
notices, fiscal years with period closing and the year-end close, controlled
bookkeeping and late-recording reviews now; manual journals, the separation of
duties with its approval exceptions report, trial balance and account totals
in later versions of this module. Other Finance modules and other Domains post
and read through this module's facade; none of them reads its tables.

## Public API

Resolve the `Ledger` service (`finance/ledger.ledger`) from the registry and
call it with a sealed `Scope` and an explicit company id. Results are plain
records (`Book`, `Account`, `Entry`, `FiscalYear`, `Period`, `ClosingEvent`,
`ControlledBookkeeping`, `LateRecording`, ...); the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `get_book` | `finance.ledger.view` | The company's currency code and minor units, or `BookNotOpen` |
| `open_book` | `finance.ledger.accounts.manage` | Opens the book once with `currency_code` (three letters, trimmed and upcased) and `minor_units` (0 to 4); a second call is `BookAlreadyOpen`. A new company's book is controlled (see Controlled bookkeeping) |
| `list_accounts` | `finance.ledger.view` | The chart of accounts by code |
| `create_account` | `finance.ledger.accounts.manage` | Adds an account: `code`, `name`, `account_type`, optional `parent_id`, `active`, `retained_earnings` |
| `update_account` | `finance.ledger.accounts.manage` | Changes `name`, `parent_id`, `active` or `retained_earnings`; `AccountNotFound` for an id of another company |
| `post_entry` | `finance.ledger.entries.post` | Posts a balanced entry of at least two lines (see Entries); `BookNotOpen` before the book is opened |
| `get_entry` | `finance.ledger.view` | One entry with its lines and the entry that reversed it; `EntryNotFound` for another company's id |
| `list_entries` | `finance.ledger.view` | Entries by date then id, 100 by default and 500 at most, paged with `after`; bounded by `from`, `to`, `account_id` and `through_entry_number`; `exclude_closing` leaves out year-end closing entries and their reversals |
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
| `Ledger::account_types`, `Ledger::late_recording_thresholds` | none | The five account types; the report's 45 and 60 days |

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
source `finance/ledger`), for the approval exceptions report of a later
version of this module.

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

## Schema (versions 001 to 004)

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
separation-of-duties tables, created here because overrides are recorded now;
the setting, `check_approval` and the reviews arrive with the controls version
of this module.

## Standards addressed

A book is opened once with its currency fixed; an account keeps its code and
type because entries are posted against them, and is deactivated only once
its balance is zero (the sum of its lines); a closed period takes no entry
until it is reopened with a reason that an independent reviewer can read, and
a locked one never reopens; a late entry is reviewed by someone other than
its poster; every write is attributed to the signed-in actor and any
impersonator through the Platform's audit capture.
