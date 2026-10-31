# Finance browser walkthrough

`finance-ui` prepares one disposable database with an ordinary tenant's recorder, independent reviewer and read-only reader, plus bills, source documents, a ledger acquisition, an asset, a bank account, receivables customers, invoices and receipts, and e-invoices backed by a local authority. It also checks the Fixed Assets list/detail flash lifecycle through their real mounted page adapters.

Run from the mounted Platform root with `DATABASE_URL` on a fresh disposable database, `BILIBI_SETTINGS_KEY` set to a fresh 64-character hex key, `FINANCE_UI_STORAGE` naming a private directory, and matching `APP_ORIGIN` and `BIND_ADDR`:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin finance-ui
```

Sign in as `manager@other.example.test`, `reviewer@other.example.test` or `viewer@other.example.test`, using the Platform walkthrough password. Tenant 2 is not the Platform operator. The startup output gives each record's URL and the acquisition/account IDs. Add `serve` to reopen the app after a code change while keeping browser-created records; retain the same storage directory and settings key. The local authority is restarted and existing e-invoices are kept.

No fixture is a production seed. The module-specific binaries remain the detailed accounting proofs; `scripts/domain-ci.sh` in the Platform is the repository gate.

## Accounting system conversion

On a fresh disposable PostgreSQL 18 database, run the composed fixture with the same private settings key and loopback origin throughout:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin conversion -- serve
```

It prepares source opening balances and setup through the owning facades, leaving the opening registers empty. Company 001 is the browser company; Company 002 is the separate MCP company. Company 004 has two receivables controls and two payables controls, including old-system controls that were never configured in Finance. Sign in as `admin@example.test` for the independent reviewer. On the Company 001 Bills, Invoices and Fixed assets lists, open each opening-register dialog and import the printed customer, supplier and asset CSVs. Open the same dialogs on Company 004's lists, preview both multi-control registers, check both account reconciliations, then cancel those previews. Inspect aging and the asset schedule, record a 1,000 customer receipt dated 2026-07-15, and submit a 500 supplier payment as `manager@example.test`, approving it as the reviewer. The conversion date is 2026-07-01. Do not run depreciation in these two companies before comparing them.

Provision private disposable OAuth credentials with the `agent-operations` binary (`BILIBI_AGENT_PROOF=disposable`, `BILIBI_AGENT_CREDENTIALS` naming a new private file), then run `conversion-operations.py` with that environment and `BILIBI_AGENT_URL` pointing to the fixture. It discovers the import contracts and guides, exercises mismatch refusals, dependent-plan rollback and exact retries, previews both Company 004 multi-control registers, proves that omitted old-system controls are named and refused, and repeats the imports and settlements in Company 002 through MCP. It compares the final account balances with Company 001 and checks that imports add no ledger entry or historical tax lines. Source setup is fixture preparation, including the asset category; it is not evidence of general asset lifecycle MCP operations.

To check all three merged lists through the normal Platform launcher, use a separate fresh disposable database and prepare it without serving:

```sh
BILIBI_OPENING_PROOF_CONFIG=/private/path/opening-sources.json \
  cargo run -p bilibi-finance-walkthrough-fixture --bin conversion -- prepare-launcher
```

Start that database with the Platform's `./bilibi start` on a unique HTTPS `.localhost` name. Sign in as `ai@agent.my` with the walkthrough password. Startup retires the old walkthrough accounts. The three record lists show their opening actions, source and status filters, and the same record pages for imported records.

To use MCP against that launched app, run the same binary on the same database with a new private credentials path and the exact launcher origin:

```sh
BILIBI_AGENT_PROOF=disposable \
APP_ORIGIN=https://your-name.localhost \
BILIBI_AGENT_CREDENTIALS=/private/path/opening-agent.json \
  cargo run -p bilibi-finance-walkthrough-fixture --bin conversion -- launcher-credentials
```

The command grants the launcher's administrator a short-lived OAuth connection and never prints its token. After approving the opening asset register in the browser, run the same binary with `launcher-ordinary` and `BILIBI_AGENT_PROOF=disposable` on the same database. It adds an ordinary asset through the Ledger and Fixed Assets facades, so the Fixed assets list shows both sources.

For the focused Receivables paging proof, use another fresh disposable database:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin conversion -- opening-list-scale
```

This mode stages and reconciles more than 2,000 opening invoices through the public facade, adds ordinary and nonmatching records, then checks filtered search, sorting, count, final-page clamping and lightweight page rows through `finance.receivables.document.page`. It exits before the full replacement lifecycle.

Run `conversion` without `serve` afterward for Company 003's facade proof:
source accumulation differing from lifetime projection, prospective depreciation,
settlement reversals, report reproduction across entryless imports, independent
approval, unauthorized-write refusal and permanent source evidence. It also
retains comparative, monthly movement, SST and payroll source schedules without
creating postings, returns, payments or tax lines. Around the staged 10,000-row
and replacement lifecycle (`src/conversion_lifecycle.rs`), it runs the immutable
conversion-report and sign-off proof described in [Conversion
reports](../../reports/README.md#conversion-reports). Use `-- report-browser` to
stop after preparing the unsigned browser report and before the bulk correction
proof. Each binary refuses ordinary production setup; these are accounting
walkthroughs, not production seeds or a historical transaction migration tool.
