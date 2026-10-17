# Payroll posting

`finance/payroll` recognises finalised payroll runs and approved staff claims in the ledger. It mirrors Bilimbi `Bilimbi.Finance.Payroll` at `4abfaf829321f8540581d833752a086b21b45ba9`. It does not calculate payroll, submit statutory returns, pay salaries or settle reimbursements. NY1 is services only; the Postings and Setup pages belong to NY2.

## People is unavailable

People is not mounted and has no published domain-event contract. `people_available()` is false. Authorized run/claim lists and run/single-claim/batch posting return `PayrollError::PeopleUnavailable` (“The payroll and claims source is not installed.”), without creating attempts or entries. Authorization happens first. Local settings, mappings, logs, evidence and correction of existing postings still work.

The captain's cross-domain rule supersedes Bilimbi's runtime `PeopleSource`: no People port, Cargo dependency, descriptor edge, SQL or invented event subscription. `source.rs` contains the `TODO(domain events)` seam for an owned projection after People documents its event names, versions and facts. Its private normalized records are accounting inputs, not a wire contract or a public arbitrary-input posting API. Successful recognition remains unproven until that integration exists; the walkthrough's historical entries prove reversal, not People posting.

## Facade and authorization

Resolve `Payroll` at `finance/payroll.payroll`. Every company-scoped call takes a sealed `Scope` and authorizes through Core Company. Actor and impersonator are derived only from that scope. The three verbatim capabilities are granted to `tenant_owner`:

| Capability | Operations |
| --- | --- |
| `finance.payroll.view` | `get_settings`, `list_mappings`, settings/mapping change logs, source lists, `list_postings`, `get_posting`, `list_posting_events`, `evidence_page` |
| `finance.payroll.settings.manage` | `put_settings`, `put_mapping` |
| `finance.payroll.postings.post` | `post_payroll_run`, `post_claim`, `post_claims`, `reverse_posting` |

Writes open `Ledger::write`, recheck the owner's capability inside the audited transaction and hold the writable-company lock. Ledger independently authorizes its reads/posting/reversal. An archived company cannot change. Every affected Payroll row is a `Change`; settings and mapping histories commit atomically with their current values.

## Posting accounts and mappings

`SettingsChange` accepts optional `net_pay_account_id`, `claims_payable_account_id` and `reason`. Both accounts are required initially, active liabilities of this company; omitted values retain existing settings. `MappingChange` upserts by the trimmed, case-sensitive `(kind, code, direction)` key. Code is 1–60 characters. Keys never change; no delete exists.

| Kind | Direction | Expense-side account | Liability-side account |
| --- | --- | --- | --- |
| `component` | `earning` | required asset or expense | absent |
| `component` | `deduction` | absent | required liability |
| `component` | `employer` | required asset or expense | required liability |
| `claim_category` | `claim` | required asset or expense | absent |

All accounts must be active and company-owned at configuration time. Optional reasons trim blank to null and allow at most 500 characters. A reason alone or unchanged account values create no history. Histories preserve old/new account IDs, actor, impersonator and database-clock time. Settings and mappings remain retained; their identities and all history/event rows are protected from change/delete/truncate.

## Accounting contract behind the unavailable source

Runs aggregate components by code/direction, resolve every mapping even for a rounded-zero amount, then round each aggregate half away from zero to the book's minor units and construct `Money`. Earnings debit expense-side accounts; deductions credit liabilities; employer amounts do both. Net pay is rounded earnings minus rounded deductions: credit the net-pay liability when positive, debit when negative, omit when zero. Employer contributions never affect net pay. Negative components are not flipped to another side; Ledger refuses invalid lines. A zero run is refused.

Claims round the approved amount the same way, debit the category account and credit claims payable. Zero/negative rounded amounts, missing category/date, missing mappings and currency disagreement are refused. There is no currency conversion. Runs use pay date, claims use the source's company-local approval date. Entry total is sum of debits, not net pay.

Entries are owned by `finance/payroll` with `payroll_run:<id>` or `claim:<id>`. `Ledger::post_entry_in` and the Payroll `posted` event share one transaction. Company locking and database unique indexes allow one original per source, including after reversal. A batch takes one source snapshot, preserves request order and posts each claim in its own transaction, returning individual outcomes; source/authorization refusal fails the whole batch before attempts.

`get_posting` takes the **posted-event ID**, not the ledger entry ID. `reverse_posting` needs a trimmed, nonblank reason of at most 500 characters. An omitted date is company-local today from Base DateTime's `localization.timezone`. It calls `Ledger::reverse_owned_entry` in the transaction recording `reversed`; Ledger enforces dates/periods and one mirror entry. Source availability is irrelevant. Originals and all failures remain retained; a later successful post/reversal clears the displayed latest failure.

Preflight and duplicate refusals create no attempt. Other accounting refusals roll back before a fresh authorized write records `posting_failed`/`reversal_failed`; this is best effort if the company became unwritable or the database cannot be reached. Structured transient SQL states (lock unavailable, serialization, deadlock, cancellation), I/O and pool failures retry once in fresh transactions, reconciling durable success before retry/failure after a potentially lost commit acknowledgement. Nontransient infrastructure errors remain errors. The pinned Platform hides the underlying driver's closed-connection flag; a connection failure with no exposed cause cannot be classified precisely and is returned rather than guessed retryable.

## Evidence and bounds

`evidence_datasets()` lists `mapping`, `mapping_change`, `posting_event`, `settings`, `settings_change`. `evidence_page(scope, company, dataset, limit, after)` exposes explicit field order, JSON rows and next cursor; IDs ascend, `limit` is 1–500 (default 500), `after` is nonnegative (default zero). A full final page returns its last ID, so the caller may need an empty follow-up. Exact decimal totals and ISO dates/times are strings. Each call is a live authorized read, not a pinned multi-page snapshot. Change/event convenience lists return their first 500 ID-ordered rows; use evidence cursors for complete history. Postings returns the latest 500 originals with standing. Mappings are ordered by kind/code/direction.

## Running proof

On an empty disposable PostgreSQL 18 database, from the mounted Platform root:

```sh
DATABASE_URL=postgres://…/disposable BILIBI_SETTINGS_KEY=$(openssl rand -hex 32) \
  cargo run -p bilibi-finance-walkthrough-fixture
```

The Payroll section asserts configuration/no-op histories, account roles, isolation and withheld writes, source `unavailable` without attempts, Ledger owner rollback and independent grants, an explicitly labelled historical posting's reasoned reversal, earlier-date failure and retry, concurrent reversal, mirrored lines, all five evidence cursors, audited affected rows, and PostgreSQL retention/uniqueness/event-shape refusals. No unit tests, provider stubs or production seeds.
