# Ledger

Module ID: `finance/ledger` (Bilimbi `Bilimbi.Finance.Ledger`). A company's
general ledger: its book, chart of accounts, journal entries and posting
notices now; fiscal years and period closing, manual journals, the separation
of duties with its approval exceptions, controlled bookkeeping, late-recording
reviews, trial balance and account totals in later versions of this module. Other Finance modules and other Domains post and read through
this module's facade; none of them reads its tables.

## Public API

Resolve the `Ledger` service (`finance/ledger.ledger`) from the registry and
call it with a sealed `Scope` and an explicit company id. Results are plain
records (`Book`, `Account`, `Entry`); the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `get_book` | `finance.ledger.view` | The company's currency code and minor units, or `BookNotOpen` |
| `open_book` | `finance.ledger.accounts.manage` | Opens the book once with `currency_code` (three letters, trimmed and upcased) and `minor_units` (0 to 4); a second call is `BookAlreadyOpen`. A new company's book is controlled (see Controlled bookkeeping) |
| `list_accounts` | `finance.ledger.view` | The chart of accounts by code |
| `create_account` | `finance.ledger.accounts.manage` | Adds an account: `code`, `name`, `account_type`, optional `parent_id`, `active`, `retained_earnings` |
| `update_account` | `finance.ledger.accounts.manage` | Changes `name`, `parent_id`, `active` or `retained_earnings`; `AccountNotFound` for an id of another company |
| `post_entry` | `finance.ledger.entries.post` | Posts a balanced entry of at least two lines (see Entries); `BookNotOpen` before the book is opened |
| `get_entry` | `finance.ledger.view` | One entry with its lines and the entry that reversed it; `EntryNotFound` for another company's id |
| `list_entries` | `finance.ledger.view` | Entries by date then id, 100 by default and 500 at most, paged with `after`; bounded by `from`, `to`, `account_id` and `through_entry_number` |
| `subscribe_postings` | `finance.ledger.view` | A `PostingSubscription` that hears each committed entry of the company (see Posting notices) |
| `Ledger::account_types` | none | The five account types every chart classifies by |

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

Period rules (an entry dated in a closed or locked period, before the first
fiscal year, or outside any period of a controlled company) arrive with the
periods version of this module; until then every date is accepted.

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

## Schema (versions 001 to 003)

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

## Controlled bookkeeping

A company that is controlled posts only into a fiscal period it has opened
and left open. The setting is the latest of the company's append-only
changes; `open_book` records the change that turns it on, so a new company is
controlled from the start. Reading and changing the setting, and the fiscal
years it governs, arrive with the periods version of this module.

## Standards addressed

A book is opened once with its currency fixed; an account keeps its code and
type because entries are posted against them, and is deactivated only once
its balance is zero (the sum of its lines); every write is attributed to the signed-in actor and any
impersonator through the Platform's audit capture.
