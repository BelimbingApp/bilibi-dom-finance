# Finance browser walkthrough

`finance-ui` prepares one disposable database with an ordinary tenant's recorder, independent reviewer and read-only reader, plus bills, source documents, a ledger acquisition, an asset, a bank account, receivables customers, invoices and receipts, and e-invoices backed by a local authority. It also checks the Fixed Assets list/detail flash lifecycle through their real mounted page adapters.

Run from the mounted Platform root with `DATABASE_URL` on a fresh disposable database, `BILIBI_SETTINGS_KEY` set to a fresh 64-character hex key, `FINANCE_UI_STORAGE` naming a private directory, and matching `APP_ORIGIN` and `BIND_ADDR`:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin finance-ui
```

Sign in as `manager@other.example.test`, `reviewer@other.example.test` or `viewer@other.example.test`, using the Platform walkthrough password. Tenant 2 is not the Platform operator. The startup output gives each record's URL and the acquisition/account IDs. Add `serve` to reopen the app after a code change while keeping browser-created records; retain the same storage directory and settings key. The local authority is restarted and existing e-invoices are kept.

No fixture is a production seed. The module-specific binaries remain the detailed accounting proofs; `scripts/domain-ci.sh` in the Platform is the repository gate.

## Accounting system conversion

On a fresh disposable PostgreSQL 18 database, run the composed fixture with the
same private settings key and loopback origin throughout:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin conversion -- serve
```

It prepares source opening balances and setup through the owning facades, leaving
the opening registers empty. Company 001 is the browser company; Company 002 is
the separate MCP company. Company 004 has two receivables controls and two
payables controls, including old-system controls that were never configured in
Finance. Sign in as `admin@example.test` for the independent reviewer. Import the
printed customer, supplier and asset CSVs through the Company 001 opening pages.
Open the Company 004 invoice and bill pages, preview both multi-control registers,
check both account reconciliations, then cancel those previews. Inspect aging and
the asset schedule, record a 1,000 customer receipt dated 2026-07-15, and submit
a 500 supplier payment as
`manager@example.test`, approving it as the reviewer. The conversion date is
2026-07-01. Do not run depreciation in these two companies before comparing them.

Provision private disposable OAuth credentials with the `agent-operations` binary
(`BILIBI_AGENT_PROOF=disposable`, `BILIBI_AGENT_CREDENTIALS` naming a new private
file), then run `conversion-operations.py` with that environment and
`BILIBI_AGENT_URL` pointing to the fixture. It discovers the import contracts and
guides, exercises mismatch refusals, dependent-plan rollback and exact retries,
previews both Company 004 multi-control registers, proves that omitted old-system
controls are named and refused, and repeats the imports and settlements in Company
002 through MCP. It compares
the final account balances with Company 001 and checks that imports add no ledger
entry or historical tax lines. Source setup is fixture preparation, including the
asset category; it is not evidence of general asset lifecycle MCP operations.

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
