# bilibi-dom-finance

Finance Domain for [Bilibi](https://github.com/BelimbingApp/bilibi): a
company's bookkeeping, ported with exact parity from Bilimbi's
[b-dom-finance](https://github.com/BelimbingApp/b-dom-finance). It is one
optional, independently versioned repository that an adopter mounts into a
Bilibi checkout; it does not build on its own.

| Module | Descriptor ID | Owns |
| --- | --- | --- |
| [`ledger/`](ledger/README.md) | `finance/ledger` | each company's book and currency, its chart of accounts, its journal entries (append-only, numbered without gaps, balanced at commit, optionally owned by another module's record) with posting notices that announce only committed entries, its fiscal years of monthly periods with the closing log (close, reopen, lock, year-end close and reopen with closing entries), controlled bookkeeping and its changes, the late-recording report with its reviews, reversals (an owned entry only through its owner), the separation of duties (impersonation counted) with the approval exceptions log and its independent reviews, manual journals that post only once someone else approves them, the trial balance and account totals (also by owner) summed by PostgreSQL at a pinned entry, and the evidence datasets of the auditor's pack. |
| [`attachments/`](attachments/README.md) | `finance/attachments` | source documents kept with the records they support: records-grade files in Base Artifacts, the company's retention years and their history, replace and remove while a record is a draft, legal holds, and the retention refresh job that follows a record's final date. The `Source` contract each record owner implements is here. |
| [`einvoice/`](einvoice/README.md) | `finance/einvoice` | a company's fiscal documents for a clearance authority: the append-only schema (documents, lines, submissions, status events, consolidated sources and source links), the `finance.einvoice` settings (environment, format version, credentials, signing certificate and key, supplier profile, posting accounts) with encryption and PEM validators, the platform API and portal URLs, the `Authority` contract with MyInvois, and the authority's code lists with company overrides. Issuing, signing, submitting, posting and the pages follow in later versions of the module. |
| [`payables/`](payables/README.md) | `finance/payables` | settings and tax codes with change histories; immutable bills and debit notes; independent approval, void, and atomic owned ledger posting; source documents. Matching, payments and pages follow in NP2–NP4. |
| [`payroll/`](payroll/README.md) | `finance/payroll` | posting accounts and component/claim mappings with their change logs, owned ledger posting and correction, attempts and five evidence datasets. People is absent: source lists and posting return `unavailable`, with a `TODO(domain events)` seam. Pages land in NY2. |
| [`receivables/`](receivables/README.md) | `finance/receivables` | setup accounts and their immutable history, numbering series, immutable-rate tax codes, customers with flat party/address fields, and unnumbered invoice/credit-note drafts with per-line tax, atomic replacement and retained discard. Issuing and payments follow in NR2; like Bilimbi this module has no pages. |

Every other Finance module of Bilimbi (fixed assets, reconciliation, SST, reports) lands beside
the ledger as a sibling module folder, in later slices of the port.

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

Its NR1 section first exercises Receivables through the real facade: setup history and no-op configuration, immutable code identities, customer address normalization, checked decimal arithmetic and per-line half-up tax, empty/replace/keep/clear/discard drafts, invalid replacement rollback, inactive references, scoped reads and writes, paging, concurrent duplicate creation, archived-company refusal, SQL guards and audit capture. See the [Receivables contract](receivables/README.md) for input fields and the NR2 boundary.

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
own journal and exception refusals. Its e-invoice section shows the code lists and
their overrides, the company settings with their validators and encryption
(it needs `BILIBI_SETTINGS_KEY`, 64 hex characters, and `openssl` for a
throwaway certificate and key), the setup read, the authority's rules and the
database's refusals on the e-invoice tables.
Its totals section reads Company 001's
trial balance (balanced, refused out of reach), posts a backdated entry and
reproduces the earlier figures from the pinned entry number, compares an
account's PostgreSQL totals with its listed lines, splits totals by owner,
shows Company 002's closed year with and without its closing entries, and
pages every evidence dataset.

The Payroll section proves the People-absent `unavailable` result without any entry or attempt, posting account/mapping validation and no-op histories, scope isolation, atomic owner-posting rollback, correction of explicitly historical fixture entries (including a race and a failed-date attempt), all five evidence datasets, audited rows and the database's retention/uniqueness refusals. It does not claim People-backed recognition while that publisher is absent.

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
