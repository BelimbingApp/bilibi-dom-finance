# Reports

Module ID: `finance/reports`, crate `bilibi-finance-reports`, ported from
Bilimbi's `Bilimbi.Finance.Reports`. A company's management reports, read live
from its general ledger: profit and loss for any date range, the balance sheet
and the cash position as of any date, headline figures against the previous
period, and the entries behind any account's figure. It stores immutable issued
snapshots; every figure comes through `finance/ledger`'s facade, and it reads
none of the ledger's tables.

Management reports and account activity are available at `/finance/reports`
and `/finance/reports/accounts/{account_id}` (NT2). Bookkeeping and Approval
exceptions are also available (NT3), alongside auditor-pack generation and downloads (NT4).

## Pages

Both routes require `finance.ledger.view` and read the signed-in account's company through the facade. Reports has four addressable views (`view=overview`, `profit-and-loss`, `balance-sheet`, `cash-position`), the shared period filter (the latest month with entries by default, see First report and layout), and previous-period comparison (`compare=none` disables it except on Overview). Custom dates stay a draft until Apply range succeeds. The display policy and browser timezone decide the preset's calendar.

`Pin entry cutoff` puts the displayed positive entry number in `through`; `Show latest figures` removes it. This is not an issued snapshot. Every account-amount link carries its column's dates and the displayed cutoff, including `closing=excluded` for profit and loss. Account entries show whole-window debit and credit totals and 100 dated entries at a time; `Next entries` retains the cutoff and the `(after_date, after_id)` cursor. Activity does not subscribe to postings. Account entries keep that open from/to window: a figure's drill-down is one column's dates, not one of the shared period presets.

Reports subscribe before the mounted read. Commit notices and missed-notice refreshes invalidate unpinned figures; bursts coalesce, and pinned figures ignore them. Custom-range drafts and table arrangement survive a posting refresh. The relay task stops with the page. If the relay cannot start, a visible warning explains periodic-refresh fallback. As in Bilimbi, zero is not a positive URL cutoff, so an empty ledger cannot be pinned.

All tables use shared FlexTable and built-in ListColumns on ListState. There is no new sort/filter/page-size control: Bilimbi fixes statement order and pages activity by cursor. Section headings, subtotals and footers remain ordered rows with the reference IDs; FlexTable's uniform body replaces Bilimbi's grouped statement table. A shared SectionHeading places the statement label above the card, clear of the customization notch. The shared "Pin entry cutoff" button (not the sidebar pin beside the title) raises a read-only page event because the Platform button has no patch-link variant. No domain CSS or JavaScript is added. When figures are available, Generate auditor pack produces an export for the selected period end, followed by Download auditor pack.

## Management reports, not statutory statements

Profit and loss, the balance sheet and the cash position are management reports
for the company's own use, not statutory financial statements: there is no cash
flow statement, no statement of changes in equity or other comprehensive
income, no notes, no consolidation and no currency translation, and no layout a
standard requires (MFRS, IFRS or another) is produced here. Every page and
export that shows them must say so.

## Public API

`Reports` is the registry service `finance/reports.reports`. Every call takes
the sealed `Scope` it was handed and an explicit company id.

| Function | Does |
| --- | --- |
| `profit_and_loss` | Income and expense for one to four periods, by section, and `net_income` |
| `balance_sheet` | Assets, liabilities and equity as of one to four dates, with `earnings`, `total_assets`, `total_liabilities_and_equity` |
| `cash_position` | The balances of the accounts the cash sections cover, as of one to four dates |
| `overview` | Headline figures for a period and `previous_period` of it |
| `account_activity` | One account's debits and credits over a window, and the entries behind them, paged |
| `subscribe` | The ledger's posting notices (`Ledger::subscribe_postings`) |
| `issue_report` | An immutable issued profit and loss, balance sheet, cash position or trial balance |
| `get_issued_report` | The stored copy, never recomputed |
| `evidence_datasets`, `evidence_page` | The issued reports for the auditor's pack |
| `previous_period` | The comparison period: as many whole months before, or as many days |

Every amount is a list with one value per column, in the order the periods or
dates were given; the first column is the reporting period and the rest its
comparatives. Every report takes `through_entry_number`.

## Authorization and company scope

Every function authorizes `finance.ledger.view` for the scope's actor and the
target company when it runs, through Core Company's `authorize_target`, and the
ledger calls it makes authorize again: a missing grant or a sibling company
without tenant-wide reach is `Unauthorized`, and a company the scope cannot see
is `CompanyNotFound`. Issuing asks for `finance.reports.issue` as well, and
runs in an audited mutation transaction holding the company's lock, so an
archived company is `CompanyArchived`. `finance.reports.reports-export` generates and downloads the auditor pack, with ledger view required. All nine source views enable the full evidence pack; otherwise it produces a clearly labelled ledger-and-reports pack. Both capabilities
are granted to `tenant_owner`.

## Figures that reproduce

A report names `through_entry_number`, the last ledger entry it counts, and
every column is read at that same number. Posted entries never change and are
numbered in posting order, so passing the number back freezes the postings
counted, including later back-dated entries. It does **not** freeze the chart,
names, sections or settings: removing an account from a cash section changes
even a pinned live cash total. `account_activity` given the report's window,
number and `exclude_closing` lists exactly the entries the figure counted, and
its debits and credits equal the figure.

Each posting account reports its own postings and a group is never a row, so
nothing is counted twice.

## Grouping and signs

The five account types are the one fixed classification. A profit and loss
lists income then expenses; a balance sheet lists assets, liabilities, then
equity. Within a type, accounts are grouped by the company's sections in their
configured order, and accounts no section covers follow under no heading
(`name: None`). A statement lists every account with a posting in any of its
columns.

Amounts are on the type's natural side: assets and expenses as debits less
credits, liabilities, equity and income as credits less debits. A profit and
loss, the overview's income and expense, and their drill-downs leave out the
ledger's year-end closing entries and their reversals (`exclude_closing`), so a
closed year still shows its activity. The balance sheet counts them: a closed
year's result sits in the retained-earnings account, and `earnings` is the
income less expense not yet closed into it. That line is part of equity, so
`total_assets` always equals `total_liabilities_and_equity`.

The cash position lists every account its sections cover, whatever its type and
whether or not it has postings, as debits less credits, so an overdrawn bank
account kept as a liability counts against the total. With no cash section
configured, the accounts are the Bank reconciliation accounts (see First report
and layout).

## Configuration

Section headings and account choices are a company's own data, never code. They
are the `finance.reports.sections` setting (Base Settings, tenant or company
scope, editor group Reports, edited by holders of
`finance.ledger.accounts.manage` on the Report sections page): an ordered JSON array of sections, each with
only these keys.

```json
[
  {"report": "profit_and_loss", "name": "<heading>", "accounts": ["<code>", "<code>"]},
  {"report": "balance_sheet", "name": "<heading>", "accounts": ["<code>"]},
  {"report": "cash_position", "name": "<heading>", "accounts": ["<code>"]}
]
```

A listed account or account group brings everything under it in the chart, unless a row under it is
listed itself: the nearest listed account decides, and within one report the
first section that lists a code keeps it. Headings (at most 100 characters) and
codes (at most 50) are trimmed. The setting's validator refuses any other shape
on save; a stored value that is not valid (written before validation, or
adopted) groups nothing rather than grouping wrongly. A report names, in
`unmatched_codes`, any listed code the chart does not have.

## Comparative periods

`previous_period` gives a period of whole calendar months the same number of
whole months before it (a quarter compares with the previous quarter, a year
with the previous year) and any other period the same number of days before
it.

## Live updates

`subscribe` relays the ledger's posting notices, which PostgreSQL delivers only
when the posting transaction commits. A notice carries no figures; the reader
reads again through this facade, and on `PostingEvent::Refresh` (notices may
have been missed) reads again as well.

## Issued reports

`issue_report(scope, company, &Issue, through_entry_number)` stores the figures
(the live report's own JSON: string keys, exact decimal strings, ISO dates),
the entry cutoff, the chart they were arranged by, the configured layout
(`{"configured_sections": ...}`), each covered account's section with the
effective sections and unmatched codes (`membership`, `{}` for a trial
balance), the implementation version, the issuer and any impersonator, and a
time the database stamps. The issuing transaction holds the company's lock,
which every ledger write takes, so the figures, chart and book it reads through
the ledger's facade describe one state of the ledger; the layout is read once
and every grouping comes from that one read. `get_issued_report` returns that
stored copy. PostgreSQL refuses update, delete and truncate of
`finance_reports_issued`. An entry pin alone reproduces postings, not a
complete report: issue one for a durable copy.

## Performance

Each column is one aggregate query in the ledger: PostgreSQL sums the lines per
account and returns one row per account. No report loads lines into memory;
the drill-down pages entries 100 at a time (1 to 500). A report reads at most
four columns, and the overview four windows.

## Proof

`fixtures/walkthrough` (the container README's Walkthrough fixture) configures
Company 001's sections, reads every report with comparatives, follows a figure
to its entries, reproduces a report from its pin after a later post its
subscription hears, issues and reads back snapshots, and shows the facade's
and the database's refusals.

From the Platform root, with `DATABASE_URL` naming a disposable PostgreSQL 18 database, `cargo run -p bilibi-finance-walkthrough-fixture --bin reports-ui` exercises the mounted page contracts and prepares the browser fixture. It proves subscribe-before-read, a stale-click pin, pinned and unpinned notices, range draft/error retention, table compactness, four views, comparison off, unopened and forbidden ledgers, and account cursors at 99/100/101 entries with stable whole-window totals. Sign in as `manager@example.test` with the walkthrough password `bilibi-dev`, then open `/finance/reports`; running the same command with `-- post` commits a real later posting to demonstrate live refresh and pinned figures. These are executable fixture proofs, not unit tests or production seeds.

## Bookkeeping and approval exceptions pages

NT3 ports Bilimbi's `/finance/reports/bookkeeping` and `/finance/reports/approval-exceptions` pages. They use the ledger facade for controlled bookkeeping, fiscal years, late-recording reviews, separation of duties and independent exception reviews. Every write checks its capability when the event arrives and again inside the ledger's audited transaction.

The pages are framed routes protected by `finance.ledger.view` and menu items in the Bookkeeping group under Finance. The Bookkeeping page links to the ledger's Fiscal years page, which also closes, reopens and locks periods. Bookkeeping keeps `status=all|approaching|late`, `review=all|pending|reviewed` and the positive `after` entry-number cursor in the URL. Approval exceptions defaults to `period=this_month`, resolves custom `from`/`to` dates against the saved display clock (including the reported browser timezone in Local mode), and keeps the same review filter and an exception-number cursor. Each page reads at most 100 rows and offers the reference's later-records link when full. FlexTable supplies column customization and compact rows for both records and change histories; these reference pages have no additional sort controls.

Turning off controlled bookkeeping requires a reason and records an approval exception atomically through `Ledger::set_controlled_bookkeeping`. Fiscal-year opening is explicit, including the first year's start date. Separation changes and both kinds of reviews use their ledger capabilities; reviews retain the ledger's independent-person and impersonation checks. Invalid reasons/dates keep their dialog open; other refusals close it and show the reference wording. As in the reference, a control the reader may not use is not rendered (no extra explanation); the fiscal-year row is hidden without the open capability. Open dialogs retain their intended separation state, so another session's change cannot invert a later submission.

Run the page-service proof on a fresh disposable database with `DATABASE_URL=… cargo run -p bilibi-finance-walkthrough-fixture --bin report-controls`. It migrates and installs only walkthrough records, then exercises the actual page service and ledger: first fiscal year, invalid inputs, Unicode reasons, controlled overrides, date/review filters, independent reviews, stale grants, review races, archived-company refusals and PostgreSQL's append-only review guards. `-- --empty` installs the empty walkthrough state for comparison with the R1 reference captures before opening any book.

The page's company is the sealed actor's affiliation, matching the other Finance pages. An account without an affiliation gets an explicit missing-company state. The page proof creates and binds the independent reviewer's Company 001 affiliation through the Core Affiliation and User facades; its administrator stays unaffiliated so this missing-context case remains covered.

Archived-company history remains readable. Until Base Settings accepts an archived company for display reads, these pages fall back to the user's tenant display clock, as the report facade already does for the unavailable company layout. The ledger still refuses every archived-company write; this display fallback only keeps that refusal visible on the live page.

## Auditor pack

`export_auditor_pack(scope, company, as_of)` always collects the complete ledger, trial balance and Reports' issued reports. When the person holds every Finance source view, it also collects every dataset published by the nine Finance siblings and the aging summaries. Otherwise it excludes all subledger/source datasets, attachment indexes and authority payloads, and labels the result as a ledger-and-reports pack. The page's success message and manifest explicitly exclude Payroll source records while retaining payroll journal lines. It never removes ledger lines or makes an incomplete trial balance appear complete. Collection goes through the authorized facades on one PostgreSQL repeatable-read transaction (write-enabled only because Base Authz records its access decisions on the caller's connection). The initial ledger total supplies the one `through_entry_number` for the general ledger, trial balance and aged receivables/payables. The database snapshot also freezes mutable account labels, document states, settings histories and attachment status across all evidence pages; an entry pin alone cannot do this. `as_of` dates the summaries only: evidence includes all company records visible at the snapshot, including drafts and reversals. A page's positive entry cutoff does not truncate the auditor pack.

The ZIP includes `manifest.json`, one `<module>-<dataset>.csv` per sibling dataset, `issued-reports.csv`, `general-ledger.csv`, JSON and CSV trial balance and aging summaries, `source-file-index.csv`, and exact `authority-payload-<submission-id>.json` files. The attachment index is `attachments-attachment.csv`: it carries filenames, hashes, retention and holds, with `import_mapping_ref` linking statement attachments to `reconciliation-statement.csv#<id>`. Original supporting documents and bank-statement bytes stay in Attachments. Every authority payload's SHA-256 must match before inclusion. General-ledger debit and credit sums must equal the ledger facade's SQL aggregates, or no artifact is published.

Manifest version 9 adds `coverage`, `excluded_sources` and `coverage_notice`, and records the tenant, company, currency, minor units, actor and impersonator, UTC capture time, PostgreSQL snapshot, entry cutoff, filters, exact-decimal encoding, and each file's SHA-256 and row count (byte size for payloads). CSV uses UTF-8, CRLF and quoted cells; formula-like text is prefixed with an apostrophe even after whitespace or controls. Known numeric fields retain unrounded decimal strings, including negative amounts. Empty datasets still have their frozen headers. Files are collected in a random 0700 temporary directory with 0600 files, paged 500 rows at a time, hashed incrementally and removed on success, failure or cancellation. The compressed archive is checked against `artifacts.max_bytes` before loading into memory.

Publication uses Base Artifacts' ordinary retained download policy (`artifacts.storage_root`, `artifacts.retention_days`, `artifacts.max_bytes`). Its private owner validates an immutable UUID reference of kind `auditor-pack` (all source views) or `ledger-pack` (ledger view), and rechecks the export capability, live company and that stored coverage's view grants on create and every read. An existing full pack cannot be downloaded by someone with only ledger access or after any source permission is revoked; generating a new restricted pack never changes that old artifact's protection. Maintenance requires `finance.reports.issue`. `read_auditor_pack(scope, company, artifact_id)` returns only bytes and a trusted filename; `/finance/reports/auditor-packs/{artifact_id}` resolves the signed-in actor's company and calls this facade on every click. Base Artifacts reauthenticates the actor, verifies expiry, byte size and hash, and commits read audit evidence before delivering bytes. Shared download delivery sets attachment disposition, private/no-store, nosniff and sandbox headers. Revoking ledger view or export refuses either kind of pack. Revoking another source view refuses an existing full pack; a newly generated pack then contains only ledger and report evidence.

The Generate event reauthorizes rather than trusting a mounted flag. Success and recoverable storage/refusal flashes use Bilimbi's wording. Period changes clear the previous download link. Artifact settings belong to Operator Settings; the reports page adds no setup fields.

Run `DATABASE_URL=… AUDITOR_PACK_PROOF_DIR=/absolute/private/task-directory cargo run -p bilibi-finance-walkthrough-fixture --bin auditor-pack` on a fresh disposable PostgreSQL 18 database. It proves a 500-row page boundary under a second session's backdated posting, checks archive membership and checksums, exact totals, source hashes, formula protection, issued evidence, missing/cross-company access, size refusal, the download adapter and revocation of every source grant against an already-mounted page. Its final section assigns the production External accountant role to two people, proves adjusting journal submission and independent approval, preserves every line of a mixed payroll/sales entry, inspects the restricted ZIP and proves immediate revocation against retained scopes and links. Append `-- serve` to keep the real app at `http://127.0.0.1:4324` for browser proof; use the walkthrough manager account.

Report and control-page feedback clears on the shared `clear-flash` event for its current message kind. A dismissed auditor-pack success stays dismissed across tab changes and refresh; an older success timeout cannot clear a newer refusal.

## First report and layout

Reports opens on the latest calendar month with entries. An explicit period in the address is preserved; a book with no entries opens on this month. The initial month is read through `finance.ledger.book.get` (`latest_entry_date`).

When no cash-position section is configured, cash uses exactly the ledger accounts mapped in Bank reconciliation, including an inactive bank account that still holds money. Descendant accounts are not included by that default. An explicit cash section, including one with no accounts, takes precedence. `Reconciliation::cash_account_ids` requires ledger view and returns only account identifiers, without bank details.

Use **Report sections** (`/finance/reports/sections`) to arrange report headings and choose accounts. This form replaces the generic JSON editor. Its `finance.reports.sections.get` and `finance.reports.sections.set` contracts serve pages and agents. Saving requires `finance.ledger.accounts.manage`; the shared transaction retains the layout review fence, audits the setting and refuses an archived company.
