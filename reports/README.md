# Reports

Module ID: `finance/reports`, crate `bilibi-finance-reports`, ported from
Bilimbi's `Bilimbi.Finance.Reports`. A company's management reports, read live
from its general ledger: profit and loss for any date range, the balance sheet
and the cash position as of any date, headline figures against the previous
period, and the entries behind any account's figure. It stores immutable issued
snapshots; every figure comes through `finance/ledger`'s facade, and it reads
none of the ledger's tables.

Management reports and account activity are available at `/finance/reports`
and `/finance/reports/accounts/{account_id}` (NT2). Bookkeeping, Approval
exceptions and the auditor pack follow in later slices.

## Pages

Both routes require `finance.ledger.view` and read the signed-in account's company through the facade. Reports has four addressable views (`view=overview`, `profit-and-loss`, `balance-sheet`, `cash-position`), the shared period filter (this month by default), and previous-period comparison (`compare=none` disables it except on Overview). Custom dates stay a draft until Apply range succeeds. The display policy and browser timezone decide the preset's calendar.

`Pin entry cutoff` puts the displayed positive entry number in `through`; `Show latest figures` removes it. This is not an issued snapshot. Every account-amount link carries its column's dates and the displayed cutoff, including `closing=excluded` for profit and loss. Account entries show whole-window debit and credit totals and 100 dated entries at a time; `Next entries` retains the cutoff and the `(after_date, after_id)` cursor. Activity does not subscribe to postings.

Reports subscribe before the mounted read. Commit notices and missed-notice refreshes invalidate unpinned figures; bursts coalesce, and pinned figures ignore them. Custom-range drafts and table arrangement survive a posting refresh. The relay task stops with the page. If the relay cannot start, a visible warning explains periodic-refresh fallback. As in Bilimbi, zero is not a positive URL cutoff, so an empty ledger cannot be pinned.

All tables use shared FlexTable and built-in ListColumns on ListState. There is no new sort/filter/page-size control: Bilimbi fixes statement order and pages activity by cursor. Section headings, subtotals and footers remain ordered rows with the reference IDs; FlexTable's uniform body replaces Bilimbi's grouped statement table. The shared pin button raises a read-only page event because the Platform button has no patch-link variant. No domain CSS, JavaScript or new write path is added. Auditor-pack controls belong to NT4 and are not shown yet.

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
archived company is `CompanyArchived`. `finance.reports.reports-export` is
declared for the auditor pack, which a later slice builds. Both capabilities
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

Each account reports its own postings, so a parent account is never counted
twice.

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
account kept as a liability counts against the total.

## Configuration

Section headings and account choices are a company's own data, never code. They
are the `finance.reports.sections` setting (Base Settings, tenant or company
scope, editor group Reports, edited by holders of
`finance.ledger.accounts.manage`): an ordered JSON array of sections, each with
only these keys.

```json
[
  {"report": "profit_and_loss", "name": "<heading>", "accounts": ["<code>", "<code>"]},
  {"report": "balance_sheet", "name": "<heading>", "accounts": ["<code>"]},
  {"report": "cash_position", "name": "<heading>", "accounts": ["<code>"]}
]
```

A listed account brings its sub-accounts in the chart, unless a sub-account is
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
