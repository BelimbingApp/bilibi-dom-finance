# Ledger

Module ID: `finance/ledger` (Bilimbi `Bilimbi.Finance.Ledger`). A company's
general ledger: its book and chart of accounts now; journal entries with
posting notices, fiscal years and period closing, manual journals, the
separation of duties with its approval exceptions, controlled bookkeeping,
late-recording reviews, trial balance and account totals in later versions of
this module. Other Finance modules and other Domains post and read through
this module's facade; none of them reads its tables.

## Public API

Resolve the `Ledger` service (`finance/ledger.ledger`) from the registry and
call it with a sealed `Scope` and an explicit company id. Results are plain
records (`Book`, `Account`); the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `get_book` | `finance.ledger.view` | The company's currency code and minor units, or `BookNotOpen` |
| `open_book` | `finance.ledger.accounts.manage` | Opens the book once with `currency_code` (three letters, trimmed and upcased) and `minor_units` (0 to 4); a second call is `BookAlreadyOpen`. A new company's book is controlled (see Controlled bookkeeping) |
| `list_accounts` | `finance.ledger.view` | The chart of accounts by code |
| `create_account` | `finance.ledger.accounts.manage` | Adds an account: `code`, `name`, `account_type`, optional `parent_id`, `active`, `retained_earnings` |
| `update_account` | `finance.ledger.accounts.manage` | Changes `name`, `parent_id`, `active` or `retained_earnings`; `AccountNotFound` for an id of another company |
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

## Schema (version 001)

`finance_ledger_books` (one per company; currency and minor units fixed for
good), `finance_ledger_accounts` (code and type kept by a trigger, a
composite `(id, company_id)` parent key so an account is never grouped under
another company's, one retained-earnings account per company, never deleted)
and `finance_ledger_bookkeeping_changes` (append-only, numbered and stamped by
the database, refusing a change that changes nothing). Books and bookkeeping
changes refuse UPDATE, DELETE and TRUNCATE.

## Controlled bookkeeping

A company that is controlled posts only into a fiscal period it has opened
and left open. The setting is the latest of the company's append-only
changes; `open_book` records the change that turns it on, so a new company is
controlled from the start. Reading and changing the setting, and the fiscal
years it governs, arrive with the periods version of this module.

## Standards addressed

A book is opened once with its currency fixed; an account keeps its code and
type because entries are posted against them, and is deactivated only once
its balance is zero; every write is attributed to the signed-in actor and any
impersonator through the Platform's audit capture.
