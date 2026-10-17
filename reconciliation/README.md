# Bank reconciliation

Module ID: `finance/reconciliation` (Bilimbi `Bilimbi.Finance.Reconciliation`).
A company's bank accounts, each mapped one-to-one to a ledger account; the
bank statements imported for them, immutable once imported; and, in later
slices, the matches between statement lines and the ledger's postings and the
reconciliations of an account as of a statement end date. This version is the
whole schema (eleven tables, fourteen trigger functions) with the facade for
bank accounts, statement import, and reconciliation opening, plus the bank account and import pages. Matching, figures, finalisation and posting charges and interest follow (`NC2`, `NC4`).

Depends on `finance/ledger` (the chart a bank account maps and the book a
statement's amounts are held to) and `finance/attachments` (the bank's own
file, kept whole as the statement's source document), Core Company and Base
Tenancy. It reads the ledger only through that facade and never its tables;
other modules call this facade and never read these tables.

## Public API

Resolve the `Reconciliation` service (`finance/reconciliation.reconciliation`)
and call it with a sealed `Scope` and an explicit company id. Results are plain
records; the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `list_bank_accounts`, `get_bank_account` | `finance.reconciliation.view` | The company's bank accounts, by name |
| `create_bank_account` | `finance.reconciliation.bank-accounts.manage` | Adds one: `name`, `ledger_account_id`, `import_mapping`, optional `charges_account_id`, `interest_account_id`, `match_window_days`, `active`, `reason` |
| `update_bank_account` | `finance.reconciliation.bank-accounts.manage` | Changes everything but the ledger account, with an optional `reason`; a change that changes nothing records nothing |
| `list_bank_account_changes` | `finance.reconciliation.view` | The change log: field, values before and after, actor, reason, time |
| `import_statement` | `finance.reconciliation.statements.import` | Imports a CSV file's bytes as a numbered statement with its lines |
| `list_statements`, `get_statement` | `finance.reconciliation.view` | Statements (without lines), and one with its lines in line order |
| `evidence_page` | `finance.reconciliation.view` | One ID-ordered page (`limit` 1 to 500, `after`) of a dataset for the auditor's pack, one per table: `bank_account`, `bank_account_change`, `statement`, `statement_line`, `match`, `match_line`, `match_entry`, `match_event`, `reconciliation_record`, `reconciliation_event`, `reconciliation_item`. Dates, times and amounts are text |
| `mapping_choices` | none | The delimiters, date formats, amount signs and decimal separators a mapping is built from |

Lists page by `PageOptions` (`limit` 1 to 500, 100 by default; `after`, the
last id read); anything else is `InvalidOption`.

The eight capability keys are Bilimbi's, verbatim, all granted to
`tenant_owner`: `finance.reconciliation.view`, `.bank-accounts.manage`,
`.statements.import`, `.matches.manage`, `.reconciliations.manage`,
`.reconciliations.finalise`, `.statement-lines.post` and
`.statement-lines.manage`. The module declares the verbs `import` and
`finalise`; `post` is the ledger's, `view` and `manage` are Base's. Every call
authorizes the capability for the scope's actor and the target company when it
runs; a write holds the company's row lock in an audited `MutationBoundary`
transaction, so an archived company is `CompanyArchived` and one company's
reconciliation writes run one at a time. A call that reads the chart (adding or
changing a bank account) or the book (importing) also needs
`finance.ledger.view`, because the ledger authorizes its own calls, and the
import needs `finance.attachments.create` for the file.

Amounts are `Money` of the book's minor units. A stored amount is read back
with `Money::new(value, MinorUnits::new(4))` (the columns are `numeric(20,4)`),
since the ledger keeps `Money::stored` private.

Errors are `ReconciliationError` variants with Bilimbi's refusal names
(`Unauthorized`, `CompanyNotFound`, `BankAccountNotFound`,
`BankAccountInactive`, `DuplicateFile`, `DuplicateLines`, `InvalidLines`,
`MissingHeaders`, `NoLines`, `TooManyLines`, `RunningBalanceMismatch`, ...);
refused values come back as `Invalid(Vec<FieldError>)`. A refusal is decided
before the audited write, and a storage or ledger refusal is passed on as
`Attachment(..)` or `Ledger(..)`.

## Bank accounts

A bank account names an active asset account of the ledger that no other bank
account maps, and keeps it for good: every match and reconciliation reads that
account's postings. It carries the expense account bank charges post to and the
income account interest posts to (each optional until needed), the window in
days a suggested match's dates may differ by (0 to 90, default 5), and its
import mapping. Its name, counter accounts, window, mapping and active flag
change in place, and every change is appended to
`finance_reconciliation_bank_account_changes` with the field, the values
before and after, the actor, the reason (trimmed, cut at 500 characters) and
the database's time. A new account's first values are logged with no old
value. A mapping is logged as JSON with its keys in order (Bilimbi logs an
Elixir `inspect` of the map). A bank account is never deleted; deactivate it,
and it imports and reconciles no more.

### Import mapping

`ImportMapping` says how the bank's CSV export is read, so a bank's layout is a
company's data, never code: the column separator, how many rows precede the
data (the last being the header row, or none), which columns hold the date,
description, optional reference, either one signed amount column (with which
sign means money in) or a debit column (money out) and a credit column (money
in), the optional running balance, the date format (nine common forms), and
the decimal separator. A column is named by its header text, or by its 1-based
position when the file has no header row. `ImportMapping::parse` builds it
from the map a company enters, filling the defaults, and refuses an unknown
setting, a missing required column, both an amount and debit or credit columns,
and a value that is not one of the choices, in Bilimbi's words. Amounts may
carry thousands separators, parentheses, a leading or trailing minus, or a
`CR` or `DR` marker.

## Statements

`import_statement` takes the file's name and bytes (and its content type,
`text/csv` unless given), reads it with the account's mapping, keeps the file
whole as the statement's source document through `finance/attachments`
(`STATEMENT_SOURCE`, `finance/reconciliation/statement`; a records-grade
document retained for the company's retention years after the end of the
financial year the statement ends in), and records one numbered statement with
its lines and the mapping it was read through in one transaction. Every check
below runs before the file is stored (the active bank account again inside the
transaction, under the company lock), and the store runs before the
transaction, as Base Artifacts asks; a refused import stores nothing, and a
storage refusal imports nothing. A statement line's `amount` is signed from the
account holder's side, money in positive and money out negative, which is the
sign of a debit less a credit on the asset account it is matched against, and
is held as `Money` in the book's minor units: a figure finer than the book's
currency is an unreadable row, not a rounded one. A row whose amount is zero is
skipped and counted in `skipped_count`.

With a running balance column, each line's balance must be the previous
line's plus its amount; a file written newest first is read in reverse, and one
that chains in neither direction is refused
(`RunningBalanceMismatch(row)`), which catches a mapping whose sign is wrong.
The statement then records its opening and closing balances. Without one, the
lines are kept in file order, oldest first (a file whose first date is after
its last is read in reverse).

The same file is refused a second time by its SHA-256
(`DuplicateFile { statement_number }`, company-wide). The same line is refused
by its fingerprint, a hash of its date, description, reference, amount, running
balance and its occurrence among identical lines of the same file, so a
statement may carry two identical movements on one day while an overlapping
export is refused whole, naming each line already on file and the statement and
line that hold it (`DuplicateLines`). Amounts are fingerprinted without
trailing zeros, so `12.5` and `12.50` are the same line (Bilimbi's text hash
tells them apart). A row the mapping cannot read refuses the file with every
faulty row and why (`InvalidLines`); a file holds at most 10,000 lines, 5 MiB
and valid UTF-8, and the reader (an RFC 4180 reader: one delimiter, quoted
fields with doubled quotes, fields spanning lines, CRLF, LF or CR line ends, a
byte-order mark) refuses a stray or unterminated quote naming the row.
Statements and lines are never changed or deleted.

`StatementSource` describes a statement to `finance/attachments` ("Bank
statement", `Statement <number>`, dated its end date, always final), so
`finance.reconciliation.view` is also needed to read its file; it reads the
statement on the connection it is handed.

## Database

The migration `finance-reconciliation-001-reconciliation` creates
`finance_reconciliation_bank_accounts`, `_bank_account_changes`, `_statements`,
`_statement_lines`, `_matches`, `_match_lines`, `_match_entries`,
`_match_events`, `_reconciliations`, `_reconciliation_events` and
`_reconciliation_items`. Rows of one company reference each other through
composite `(id, company_id)` foreign keys; ledger account and entry ids are
plain columns. PostgreSQL enforces the rules the facade checks, so no other
writer can break them:

- a bank account keeps its company and ledger account and is never deleted;
  one bank account per ledger account and per name;
- every other table is append-only: no UPDATE, DELETE or TRUNCATE;
- triggers number each company's statements, matches, match events,
  reconciliations and reconciliation events without gaps and stamp their times,
  replacing whatever the writer sent;
- the same file and the same line fingerprint are refused per company and per
  bank account by unique indexes; a statement's lines are inserted only in its
  own transaction, and a deferred trigger refuses a commit in which the line
  count or dates disagree (it does the work once for the statement and once
  for the line numbered last, not once per line, so a 10,000 line statement
  commits in about a second);
- a match's lines and entries are inserted only in its own transaction; a line
  or entry in another standing match is refused; a deferred trigger refuses a
  commit in which the two sides differ, either is empty, or the `matched` event
  is missing; an `unmatched` event needs a reason and a standing match that no
  finalised reconciliation counted;
- one open reconciliation per bank account, each ending after the last
  finalised one; a `finalised` event carries its figures and numbers and a zero
  difference; an `abandoned` one its reason; items are inserted only in the
  transaction that finalises.

A statement's `import_mapping` is NOT NULL here: a fresh Bilibi schema has no
statement imported before the mapping was kept (Bilimbi's column is nullable
for those).

## Not ported yet

- Matching, reconciliation figures/finalisation and posting charges and interest (`NC2`); the matching and reconciliation detail pages (`NC4`).

## Bank account pages (NC3)

`/finance/reconciliation` contributes the **Bank reconciliation** menu item and shows the company's accounts in a FlexTable. Its add/change form keeps the ledger mapping fixed after creation, selects active accounts of the appropriate type, and edits the bank's CSV mapping. Edit and activation events recheck `finance.reconciliation.bank-accounts.manage`; the facade checks it again in `Ledger::write`. Reads of the chart inside that write use `Ledger::list_accounts_on` on the handed transaction.

`/finance/reconciliation/accounts/{id}` shows statements newest first, their bank files through the authorized Attachments download route, and reconciliations newest first. The live upload accepts one CSV/text file up to 5 MiB and consumes its bytes once in the import event. A successful import reports the dates, line count and skipped zero rows. The next reconciliation form suggests the latest statement's end date and closing balance while preserving an entered draft. Withheld controls name the missing permission; facade refusals preserve the live page and form.

The account page's `list_reconciliations` and `open_reconciliation` facade calls are implemented in private `src/opening.rs`: openings use the book's `Money`, refuse a second open reconciliation or a date at/before the last finalised date before mutation, and audit both the record and its opening event in one company-locked write. Matching links and the successful opening's destination are the NC4 routes; that slice supplies those pages.

The bank account, statement and reconciliation lists all use Base UI's FlexTable and ListColumns for column arrangement and compact rows. Bilimbi enables no list filter, sort or pager on these pages, so the source ordering and facade bounds are retained. Statement reads traverse every 500-row facade page; attachment lookups use batches of at most 100 statement IDs.


Run the NC3 disposable browser proof from the mounted Platform with an empty database and `NC3_STORAGE_ROOT` naming a private task directory:

```sh
DATABASE_URL=postgres://postgres@localhost:5433/bilibi_bilibi_dom_nc3_ui \
APP_ORIGIN=http://localhost:4324 BIND_ADDR=127.0.0.1:4324 \
NC3_STORAGE_ROOT=/tmp/nc3-source-documents \
cargo run -p bilibi-finance-walkthrough-fixture --bin bilibi-finance-reconciliation-ui
```

It proves the facade's import/skipped rows, duplicate-file refusal, original download bytes, denied/restricted scopes, Money precision, and concurrent opening refusal before serving the app. The walkthrough's manager, staff and viewer accounts exercise full control, import-only and read-only states; use the fixture's `PASSWORD`. The operating account is left ready for browser imports and opening, and the reserve account has one open reconciliation. These are disposable fixture facts, never production seeds.
