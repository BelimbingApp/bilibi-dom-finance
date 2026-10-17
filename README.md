# bilibi-dom-finance

Finance Domain for [Bilibi](https://github.com/BelimbingApp/bilibi): a
company's bookkeeping, ported with exact parity from Bilimbi's
[b-dom-finance](https://github.com/BelimbingApp/b-dom-finance). It is one
optional, independently versioned repository that an adopter mounts into a
Bilibi checkout; it does not build on its own.

| Module | Descriptor ID | Owns |
| --- | --- | --- |
| [`ledger/`](ledger/README.md) | `finance/ledger` | each company's book and currency, its chart of accounts, its journal entries (append-only, numbered without gaps, balanced at commit, optionally owned by another module's record) with posting notices that announce only committed entries, its fiscal years of monthly periods with the closing log (close, reopen, lock, year-end close and reopen with closing entries), controlled bookkeeping and its changes, the late-recording report with its reviews, reversals (an owned entry only through its owner), the separation of duties (impersonation counted) with the approval exceptions log and its independent reviews, and manual journals that post only once someone else approves them. |
| [`attachments/`](attachments/README.md) | `finance/attachments` | source documents kept with the records they support: records-grade files in Base Artifacts, the company's retention years and their history, replace and remove while a record is a draft, legal holds, and the retention refresh job that follows a record's final date. The `Source` contract each record owner implements is here. |
| [`reconciliation/`](reconciliation/README.md) | `finance/reconciliation` | each company's bank accounts mapped one-to-one to a ledger account, with a change log and a per-account CSV import mapping (a bank's layout is the company's data), and the bank statements imported through it: read by a small RFC 4180 reader, kept whole as a source document through `finance/attachments`, numbered, immutable, and refused a second time by file SHA-256 or by line fingerprint. The schema for matches and reconciliations is in place; their services and the pages follow. |

Every other Finance module of Bilimbi (receivables, payables,
e-invoice, fixed assets, payroll, SST, reports) lands beside the ledger as a
sibling module folder, in later slices of the port.

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
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture
```

It migrates a fresh disposable database, installs the fixtures, and prints
the facade's answers to accepted and refused books, accounts, entries (owners,
settlement accounts, concurrent posts, paged reads), reach (grants,
affiliation, tenancy, archival), money finer than the book allows, the
database's own refusals (edits to posted entries, unbalanced or lineless
entries, cross-company keys) and the posting notices a commit, a rollback and
a refusal do and do not send. Its attachments section does the same for
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
own journal and exception refusals. Its reconciliation section, on Company 001
after the others, adds bank accounts (refused for a liability account, an
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
