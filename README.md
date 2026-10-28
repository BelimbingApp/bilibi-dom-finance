# bilibi-dom-finance

Finance Domain for [Bilibi](https://github.com/BelimbingApp/bilibi): a
company's bookkeeping, ported with exact parity from Bilimbi's
[b-dom-finance](https://github.com/BelimbingApp/b-dom-finance). It is one
optional, independently versioned repository that an adopter mounts into a
Bilibi checkout; it does not build on its own.

| Module | Descriptor ID | Owns |
| --- | --- | --- |
| [`ledger/`](ledger/README.md) | `finance/ledger` | each company's book and currency, its chart of accounts, its journal entries (append-only, numbered without gaps, balanced at commit, optionally owned by another module's record) with posting notices that announce only committed entries, its fiscal years of monthly periods with the closing log (close, reopen, lock, year-end close and reopen with closing entries), controlled bookkeeping and its changes, the late-recording report with its reviews, reversals (an owned entry only through its owner), the separation of duties (impersonation counted) with the approval exceptions log and its independent reviews, manual journals that post only after approval (their submitter may withdraw or correct one that is still waiting), the trial balance and account totals (also by owner) summed by PostgreSQL at a pinned entry, and the evidence datasets of the auditor's pack. |
| [`attachments/`](attachments/README.md) | `finance/attachments` | source documents kept with the records they support: records-grade files in Base Artifacts, the company's retention years and their history, replace and remove while a record is a draft, legal holds, and the retention refresh job that follows a record's final date. The `Source` contract each record owner implements is here, and so are the Source documents page, its download route and the panel a record's page embeds. |
| [`einvoice/`](einvoice/README.md) | `finance/einvoice` | a company's fiscal documents for a clearance authority: the append-only schema (documents, lines, submissions, status events, consolidated sources and source links), the `finance.einvoice` settings (environment, format version, credentials, signing certificate and key, supplier profile, posting accounts) with encryption and PEM validators, the platform API and portal URLs, the `Authority` contract with MyInvois and its rules (the draft a caller asks to issue and its validation with derived amounts, the UBL 2.1 JSON builder with its byte-exact ordered writer, the JSON XAdES signer and verifier, the certificate's facts), and the authority's code lists with company overrides. Issuing, submitting, posting and the pages follow in later versions of the module. |
| [`payables/`](payables/README.md) | `finance/payables` | settings and tax codes with change histories; immutable bills and debit notes; independent approval, void, and atomic owned ledger posting; source documents. Payment requests, independent payment approval/rejection, owned reversals, aged balances, tax totals, control-account tie-out and evidence. The three-way match (held bills, match events and exceptions, `approve_match_exception`, `billed_quantities`) against its own copy of Supply Chain's order lines and goods receipts, kept from Supply Chain's domain events and unavailable while Supply Chain is not mounted. Pages: bills (NP4), then recording a bill or debit note, payments, aged payables and setup (NP5). |
| [`payroll/`](payroll/README.md) | `finance/payroll` | the Payroll and claims Postings page and posting-account Setup page, component/claim mappings with their change logs, owned ledger posting and correction, attempts and five evidence datasets. People is absent: the page states this and source posting returns `unavailable`, with a `TODO(domain events)` seam. |
| [`receivables/`](receivables/README.md) | `finance/receivables` | setup, series, tax codes and customers; invoice/credit-note drafts and numbered issuance with frozen party and owned posting; fully allocated receipts and owned reversals; dated/pinned aging, tax and received-tax totals, control-account tie-out, attachment sources and eleven evidence datasets. Its pages (customers, invoices, receipts, aging, settings) call the same operation contracts as the agent doors; Bilimbi has none. |
| [`fixed_assets/`](fixed_assets/README.md) | `finance/fixed_assets` | immutable category revisions, acquisition from posted bill or entry debit lines, frozen assumptions and straight-line/reducing-balance schedules with full first-month and month-end recognition. Runs/corrections and pages follow in NX2/NX3. |
| [`reports/`](reports/README.md) | `finance/reports` | a company's management reports read through the ledger's facade: profit and loss, the balance sheet and the cash position with up to four comparative columns, the overview against the previous period, the entries behind any figure, all at one pinned entry number; the `finance.reports.sections` layout setting with its validator; the ledger's posting notices for live reports; and immutable issued snapshots. Its pages and the auditor pack follow in later versions of the module. |
| [`reconciliation/`](reconciliation/README.md) | `finance/reconciliation` | each company's bank accounts mapped one-to-one to a ledger account, with a change log and a per-account CSV import mapping (a bank's layout is the company's data), and the bank statements imported through it: read by a small RFC 4180 reader, kept whole as a source document through `finance/attachments`, numbered, immutable, and refused a second time by file SHA-256 or by line fingerprint. The schema for matches and reconciliations is in place; their services and the pages follow. |

Every other Finance module of Bilimbi (SST) lands beside
the ledger as a sibling module folder, in later slices of the port.

The ledger also contributes 25 agent operations through Bilibi's shared `/api/v1` and `/mcp`: eight reads and seventeen reviewed writes for books, accounts, journals, reversals, fiscal closing and the trial balance. The [ledger contract](ledger/README.md#agent-operations) and [agent guide](ledger/guide.md) describe their inputs and controls. Bilimbi's ledger has no pages. This port adds them, so a person without an AI agent can do the same daily work: the book, the chart of accounts, journal entries and reversals, manual journals, fiscal years and periods, and the trial balance ([ledger pages](ledger/README.md#pages)). Each page calls the same operation as an agent does. Every Finance operation (ledger, payables, receivables, e-invoice) serves every door (`channels: every`) and lists no step-up: Finance declares no Base Step-up operation, as Bilimbi has none.

## Mount

Clone this repository into a Bilibi checkout at `apps/domains/finance` (a real
nested checkout, never a symlink), then work from the Bilibi root:

```sh
git clone https://github.com/BelimbingApp/bilibi-dom-finance.git apps/domains/finance
python3 scripts/discover-modules.py
cargo build -p bilibi-web
DATABASE_URL=… ./target/debug/bilibi-web migrate
```

Bilibi discovers the mounted container from `bilibi.container.toml`; no list
names it. Removing the directory removes Finance's code from the next build and
keeps its data: the migration ledger keeps an unmounted module's applied
versions. The composition rules are in Bilibi's `apps/domains/AGENTS.md`.

## Walkthrough fixture

`fixtures/walkthrough/` is an example-only crate (never a product dependency)
that installs the Platform's walkthrough fixture and then opens Company 001's
book, chart of accounts and first fiscal year through the ledger facade. Its binary is the
running proof of this repository's facade:

```sh
DATABASE_URL=postgres://…/disposable BILIBI_SETTINGS_KEY=$(openssl rand -hex 32) \
  cargo run -p bilibi-finance-walkthrough-fixture
```

The settings key is an ephemeral 32-byte key for the encrypted e-invoice settings proof; never reuse a production key.

Its NR1 section first exercises Receivables setup and drafts through the real facade. Its NR2 section runs last: issue-time snapshots and numbering races, multi-control receipts, owned reversals, inactive-control settlements, closed-period and late audit rollback, dated/pinned aging and tax boundaries, unrounded received-tax shares, competing payments/reversals, control tie-out differences, both attachment sources, eleven evidence exports, live grants, archival, SQL guards and complete row audits. See the [Receivables contract](receivables/README.md) for inputs and parity adaptations.

It migrates a fresh disposable database, installs the fixtures, and prints
the facade's answers to accepted and refused books, accounts, entries (owners,
settlement accounts, concurrent posts, paged reads), reach (grants,
affiliation, tenancy, archival), money finer than the book allows, the
database's own refusals (edits to posted entries, unbalanced or lineless
entries, cross-company keys) and the posting notices a commit, a rollback and
a refusal do and do not send. The `bilibi-finance-walkthrough-ui` binary serves the composed app on a
disposable database with a few kept files and two walkthrough records (one
draft, one final) whose page mounts the attachments panel, so the Source
documents page, downloads and panel can be tried in a browser (its header
says how). Its attachments section does the same for
source documents: the company's retention years, attaching and capturing
files, replacing and removing on a draft record, a final record that keeps its
files, retention extensions and legal holds (including a change made in Base
Artifacts out of band), the retention refresh job as its system principal
before and after a tenant grants it, evidence pages, restricted sources, an
archived company and the database's own refusals. Its periods section, on
Company 002 with dates counted back from today, opens fiscal years, closes,
locks and reopens periods in and out of order, closes and reopens a year with
its closing entry and reversal, turns controlled bookkeeping off and on,
reviews entries recorded late, races a close against posts into the same
period, and ends with the database's own closing refusals. Its controls section, on
Company 001 in 2026, reverses manual entries (refused twice, for a reversal,
before its original, for a closing entry), corrects an owned entry only through
its owner's own transaction (rolled back and committed), submits, approves and
rejects journals (refused for the submitter, for an operator who impersonated
the submitter, after a period closed and an account was deactivated), turns
the separation of duties off and on around sibling approvals and overrides,
lists and reviews the approval exceptions (refused for the approver and
preparer), races two approvals of one journal, and ends with the database's
own journal and exception refusals. Its e-invoice section shows the code lists and
their overrides, the company settings with their validators and encryption
(it needs `BILIBI_SETTINGS_KEY`, 64 hex characters, and `openssl` for a
throwaway certificate and key), the setup read with the certificate's facts,
the authority's shapes and rules, drafts accepted with their derived amounts
and refused in Bilimbi's words (invoices, notes and their originals,
self-billed and consolidated documents), the payload a recorded document
becomes (unsigned, and signed, verified and tampered with), and the
database's refusals on the e-invoice tables. The e-invoice crate's
`sdk_samples` example rebuilds the authority's four SDK samples byte for byte
and verifies their signatures (see `einvoice/README.md`).
Its totals section reads Company 001's
trial balance (balanced, refused out of reach), posts a backdated entry and
reproduces the earlier figures from the pinned entry number, compares an
account's PostgreSQL totals with its listed lines, splits totals by owner,
shows Company 002's closed year with and without its closing entries, and
pages every evidence dataset. Its reports section configures Company
001's report sections (refused shapes first), reads profit and loss, the
balance sheet, the cash position and the overview with comparative columns,
follows a figure to the entries behind it, reproduces a report from its pinned
entry number after a later post its posting subscription hears, issues and
reads back immutable snapshots (refused without the capability and for an
archived company), and ends with the facade's and the database's refusals.

The Payroll section proves the People-absent `unavailable` result without any entry or attempt, posting account/mapping validation and no-op histories, scope isolation, atomic owner-posting rollback, correction of explicitly historical fixture entries (including a race and a failed-date attempt), all five evidence datasets, audited rows and the database's retention/uniqueness refusals. It does not claim People-backed recognition while that publisher is absent.

The separate NX1 walkthrough runs on its own fresh disposable database with `cargo run -p bilibi-finance-walkthrough-fixture --bin fixed-assets`. It proves immutable category history and frozen assumptions, bill-line and entry-line eligibility, exact schedules and month boundaries, duplicate registration races, grants/reach/archive refusals, evidence paging, audited rows and PostgreSQL guards. See the [Fixed Assets contract](fixed_assets/README.md). It creates no production data; there are no fixed-assets pages until NX3.

The reconciliation section runs last, on Company 001. It adds bank accounts (refused for a liability account, an
unknown or inactive account, a counter account of the wrong type, a window of
91 and twelve kinds of unreadable import mapping; changed with a reason and
read back from the change log) and imports statements in four bank layouts
(a signed amount with a running balance, written newest first; debit and credit
columns with semicolons, decimal commas and two header rows; columns by
position with no header row; month names with `CR` and `DR` markers), refusing
a file twice, an overlapping export naming the lines it repeats, unreadable
rows, missing headers, a running balance that does not follow, stray and open
quotes, a file that is not UTF-8 or is too large, a viewer, another tenant and
a deactivated account; it reads the kept file back from Attachments, races two
sessions importing one file, imports a 10,000 line statement, and ends with the
database's own statement, match and reconciliation refusals.

## Naming

Optional Bilibi repositories are named `bilibi-<role>-<id>`: `dom-` for a
Domain. The mount folder is the container ID in snake_case, so
`bilibi-dom-finance` mounts at `apps/domains/finance`. Module crates are
`bilibi-finance-<module>`, capability keys `finance.<module>.*`, tables
`finance_<module>_*` and migrations `finance-<module>-NNN-<slug>`, all as
Bilimbi names them.

## CI

CI is Bilibi's reference Domain workflow (`docs/domain-ci/ci.yml`): it checks
out Bilibi at the revision in [`.github/bilibi-revision`](.github/bilibi-revision),
mounts this repository, and runs `scripts/domain-ci.sh` over the composed
workspace (discovery, committed CSS, Cargo.lock pinning, fmt, clippy with
`-D warnings`, the locked release build, migrate, seed-production and a boot
that loads `/`). Bump that revision to build against a newer Platform.

## License

[MIT](LICENSE).
