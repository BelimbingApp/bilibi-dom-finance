# Finance browser walkthrough

`finance-ui` prepares one disposable database with an ordinary tenant's recorder, independent reviewer and read-only reader, plus bills, source documents, a ledger acquisition, an asset, a bank account and e-invoices backed by a local authority. It also checks the Fixed Assets list/detail flash lifecycle through their real mounted page adapters.

Run from the mounted Platform root with `DATABASE_URL` on a fresh disposable database, `BILIBI_SETTINGS_KEY` set to a fresh 64-character hex key, `FINANCE_UI_STORAGE` naming a private directory, and matching `APP_ORIGIN` and `BIND_ADDR`:

```sh
cargo run -p bilibi-finance-walkthrough-fixture --bin finance-ui
```

Sign in as `manager@other.example.test`, `reviewer@other.example.test` or `viewer@other.example.test`, using the Platform walkthrough password. Tenant 2 is not the Platform operator. The startup output gives each record's URL and the acquisition/account IDs. Add `serve` to reopen the app after a code change while keeping browser-created records; retain the same storage directory and settings key. The local authority is restarted and existing e-invoices are kept.

No fixture is a production seed. The module-specific binaries remain the detailed accounting proofs; `scripts/domain-ci.sh` in the Platform is the repository gate.
