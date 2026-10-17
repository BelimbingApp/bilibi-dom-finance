# Fixed assets

`finance/fixed_assets` mirrors Bilimbi's `Bilimbi.Finance.FixedAssets` at `4abfaf829321f8540581d833752a086b21b45ba9`: immutable categories and acquisitions (NX1), monthly depreciation, corrections, reconciled reports and evidence (NX2). The four NX3 pages compose shared Base UI components and contribute the three reference menu items. This module owns no CSS or JavaScript.

## Facade

Resolve `FixedAssets` at `finance/fixed_assets.fixed_assets`. Every data call takes a sealed `Scope` and company ID and checks company reach and capability. Writes use `Ledger::write`: capability checks before and inside the audited `MutationBoundary`, writable-company row lock, then every affected record captured as a `Change`. Actors and impersonators come only from the scope. Reads of source records use the providers' authorized `_on` facade methods on the caller's connection, so no write waits for a second pool slot while holding the company lock.

| Call | Capability | Result |
| --- | --- | --- |
| `create_category(scope, company, name, RevisionInput)` | `finance.fixed-assets.finance-asset-configure` | Category and first revision atomically; returns the revision with its category name. |
| `revise_category(scope, company, category_id, RevisionInput)` | same | Complete new revision; never a patch or rename. |
| `list_categories(scope, company)` | `finance.fixed-assets.view` | Name-ordered categories, latest revision and full ID-ordered history. |
| `acquire_from_bill(scope, company, bill_id, line_number, AcquisitionInput)` | `finance.fixed-assets.finance-asset-acquire` | Asset registered from a posted supplier bill's net debit line. Also requires Payables and Ledger view. |
| `acquire_from_entry(scope, company, entry_id, line_number, AcquisitionInput)` | same | Asset registered from a live original debit. Also requires Ledger view. |
| `get_asset(scope, company, id)` | `finance.fixed-assets.view` | Asset, original category revision, projected schedule, posted charges with their earliest reversal/void, asset events and permanent void state, all on one repeatable-read snapshot. Also requires Ledger view to read book precision. |
| `depreciate(scope, company, month)` | `finance.fixed-assets.finance-asset-depreciate` | One immutable monthly run, items and skipped-asset exceptions; one owned ledger entry unless empty. |
| `reverse_run(scope, company, run_id, EventInput)` | `finance.fixed-assets.finance-asset-reverse` | Reverse the latest live charges without reversing previously voided assets twice. |
| `dispose(scope, company, asset_id, DisposalInput)` | `finance.fixed-assets.finance-asset-dispose` | Sale or write-off, with an owned posting and immutable event. |
| `reverse_disposal(scope, company, asset_id, EventInput)` | `finance.fixed-assets.finance-asset-reverse` | Reverse a standing disposal through its ledger owner. |
| `void_acquisition(scope, company, asset_id, EventInput)` | `finance.fixed-assets.finance-asset-void` | Permanently retire a reversed acquisition and mirror only its live depreciation. |
| `list_runs(scope, company)` / `list_events(scope, company, asset_id)` | `finance.fixed-assets.view` | Runs newest month first with items, exceptions and reversal state; asset events in event-number order. |
| `register_report(scope, company, date, ReportPins)` / `depreciation_schedule_report(...)` | `finance.fixed-assets.view` | Same dated register, projected schedule and ledger reconciliation, at four immutable boundaries. |
| `evidence_page(scope, company, dataset, limit, after)` | `finance.fixed-assets.view` | Explicit frozen fields, exact decimal text, UTC timestamps; ID cursor and limit 1–500. Datasets: `asset`, `category`, `category_revision`, `event`, `run`, `run_exception`, `run_item`. |

All seven reference capability keys and six `finance-asset-*` verbs are declared and granted to `tenant_owner`. Reads that need ledger context also require Ledger view; nonempty postings and reversals independently require Ledger post authority.

## Frozen defaults and acquisition

`RevisionInput` requires cost and accumulated **asset** accounts (different IDs), an **expense** account, an **income** gain account and an **expense** loss account. Each must be active and belong to the company when the revision is recorded. Expense and loss may share an account. `reason` is trimmed, nonblank and at most 500 characters. Category `name` is trimmed, nonblank, company-unique and at most 200 characters.

`Assumptions` contains `method` (`StraightLine`/`ReducingBalance`, stored as `straight_line`/`reducing_balance`), `life_months` (1–1200), and `annual_rate` (annual percentage, greater than 0 through 100, at most four decimal places). Straight line clears the rate. Reducing balance requires it.

`AcquisitionInput` takes a revision, nonblank description (maximum 500 characters), in-service date, optional residual `Money`, and optional method/life/rate overrides. Absent assumptions inherit the chosen revision; residual defaults to zero. The service checks residual against book minor units, zero and cost; it never rounds input. In-service date cannot precede acquisition, and the final schedule month must fit the date type. An asset keeps the selected revision and its copied/overridden assumptions forever; later revisions and account deactivation do not rewrite them. An old revision can still register a historical debit after its accounts are deactivated.

Both source paths derive cost, acquisition date, entry ID, entry number and line number from the ledger's answer. The line must debit the revision's cost account with zero credit, and its entry must be neither reversed nor a reversal. Bill acquisition requires a posted `bill`, uses its line number in the posted entry, and retains bill provenance. One **whole debit line** supports at most one asset across both paths: no quantity splitting, tax-inclusive recalculation or allocation of additional costs. Registering posts no entry and does not claim or change source ownership; later source reversal remains possible.

As in the reference implementation, the entry path does **not** restrict the entry's owner or infer bill provenance, despite the reference calling this the manual-journal path. It accepts any otherwise qualifying original debit. This preserves the source's behavior rather than introducing a new ownership rule.

## Month convention

- Exactly `life_months` schedule rows, including zero rows, labelled with the month's first day.
- Full charge in the in-service calendar month, even for a last-day start; no daily proration.
- Straight line spreads `cost − residual` equally over useful life.
- Reducing balance charges opening carrying value × annual percentage / 1200, not remaining depreciable value or an effective compound rate.
- Half-up round each charge at the book's minor units, cap at remaining depreciable cost, floor at zero. The final month clears the entire remaining depreciable cost for either method.
- `AssetDetail::scheduled_depreciation(as_of)` includes a month only on/after its last day. These are projected figures, not posted charges or a ledger tie-out.

Examples: cost 100.01 over two months gives 50.01 then 50.00. Cost 1200, residual 100, 24% reducing balance over three months gives 24, 23.52, then 1052.48. The large final true-up is intentional reference policy.

## Monthly runs and corrections

A new run requires a first-of-month date and an existing open fiscal period, even if no charge will post. It charges cumulative scheduled depreciation through month-end minus posted, unreversed charges: missed or reversed months catch up in a later month. A reversed acquisition becomes a persisted exception (`reason = acquisition_reversed` in the run view), not a failure of the other assets. Exceptions contain only run/asset identities in storage and evidence, matching Bilimbi.

The company lock and unique company/month serialize concurrent first creation. Retrying an existing month returns its original items and exceptions plus current reversal state before period/chronology checks; empty, skipped-only and reversed months remain consumed forever. Authorization and writable-company checks still run on every retry.

`EventInput` requires an effective date and trimmed, nonblank reason of at most 500 characters. New run/correction dates cannot precede any stored run's month-end or any event's effective date, including empty/reversed runs and old reversal events. Run reversal first refuses later unreversed runs with non-voided charges and **any standing disposal in the company**, then a date before the original month-end, then later historical activity. Reverse disposals first, then live runs newest to oldest.

Disposal requires a live acquisition, no prior standing disposal, a date on/after acquisition and exact posted depreciation through the last completed month (including the disposal month if dated month-end). Proceeds must be nonnegative book-precision `Money`; positive proceeds require an active company asset settlement account, while zero ignores it. The entry removes cost and accumulation, records proceeds and posts the gain or loss. A disposal reversal restores the asset for later catch-up.

Void requires a reversed acquisition and no standing disposal. It requires an open period even when no depreciation exists, mirrors only the asset's charges in unreversed runs with one ledger-line pair per original charge, and remains permanent. It leaves the asset visible with zero scheduled/accumulated depreciation and carrying value and excludes it from reconciliation. Whole-run reversals retain owner `run:<id>`; a mixed void/live run mirrors only live charges with owner `event:<id>`. Disposals and void mirrors also use their event owner; all records and entries commit in the same audited transaction. Ordinary mirrors require active posting accounts; full owner reversals may reverse inactive original accounts, as in the reference. Void dates are not additionally restricted by the source reversal's date.

## Reproducing and reconciling reports

Pass the same date and **all four returned pins**: `through_entry_number`, `through_asset_number`, `through_event_number` and `through_category_revision_id` (or `report.pins()`). A negative pin is refused. The register bounds acquisitions, category revisions, events and run entries independently, so a later backdated registration or entryless void cannot rewrite a reproduced figure. Lifecycle comes from bounded events, not today's ledger reversal links. Full projected schedules remain available for disposed assets.

Reports read on one repeatable-read connection; write workflows use that same report implementation on their company-locked transaction. The transaction is not SQL `READ ONLY` because Platform authorization records decision logs. No report writes business data. Ledger's authorized `account_totals_on` and `list_fiscal_years_on` avoid another pool checkout while the company lock is held.

Reconciliation includes accounts from every bounded category revision, including unused ones. Cost and accumulated depreciation use ordinary debit-positive ledger balances; expense/gain/loss use activity totals excluding year-end closing entries and their reversals at the same entry pin. Shared accounts aggregate. Each row gives actual ledger balance, register balance and `difference = ledger − register`. Unregistered acquisitions, unrelated postings and reversed-but-not-yet-voided acquisitions remain visible differences. Disposed assets retain depreciation expense and disposal gain/loss, but no cost/accumulated balance.

## Storage and refusals

Migration `finance-fixed-assets-001-acquisitions` creates the three NX1 tables. PostgreSQL assigns company-local asset numbers under its advisory transaction lock, stamps UTC recording time, enforces scoped composite references and unique source debits, and refuses UPDATE, DELETE and TRUNCATE permanently. No sibling SQL or cross-domain dependency is used.

Migration `finance-fixed-assets-002-runs-corrections` adds runs, run items, run exceptions and events with the same retention, scoped references and audit identity. The database numbers company events, enforces event shapes/transitions and one reversal/void, and seals children when their run's transaction commits. Deferred constraints on both run and item insertions enforce `total = sum(items.amount)`, including after an early `SET CONSTRAINTS`. This closes a reference SQL integrity gap without changing valid facade behavior. Evidence keeps frozen explicit fields and ID-ordered pagination, including nullable entry IDs for empty runs and entryless voids.

Expected refusals are `FixedAssetsError` variants (`BillNotPosted`, `BillLineNotFound`, `EntryLineNotFound`, `NotAcquisitionDebit`, `AcquisitionReversed`, reach/archive/not-found) or `Invalid(Vec<FieldError>)`. Duplicate names and sources are field refusals decided before mutation; provider failures retain their owning error. Two deliberate corrections to reference defects: blank revision reasons are refused before SQL; SQL checks require a non-null reducing-balance rate and both bill-source fields together instead of allowing nullable `CHECK` expressions to pass.

## Running proof

From the mounted Platform root, on a fresh disposable database:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture --bin fixed-assets
```

The binary migrates and installs the disposable walkthrough fixture, then proves category histories, defaults frozen after revision, book precision, source eligibility, posted-bill line provenance and ownership, straight-line ties, reducing balance with residual/final true-up, caps and zero rows, leap-month labels, historical inactive accounts, concurrent duplicates, reversal eligibility, revoked provider/owner grants, company scope, archive refusal, paged evidence, audited rows and PostgreSQL retention/uniqueness/check/composite-key refusals. It adds no production seeds and no unit tests. The repository gate remains the Platform's `scripts/domain-ci.sh`.

On a second fresh disposable database, run NX2:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture --bin fixed-assets-runs
```

This proves concurrent first-run creation/retries, posting-authority rollback, asymmetric 100/50 charges, void/partial/whole reversals and catch-up, gain/loss/break-even/write-off disposal, unrelated standing-disposal dependencies, historical reproduction, year-end closing/reopening reconciliation, skipped-only/empty consumed months, closed-period entryless void refusal, scope/archive refusals, all seven evidence datasets, audited business rows and SQL immutability/sealing/aggregate guards. It exits with `NX2: runs, corrections, reports, evidence, audit and database fences passed`.


## Pages

The view capability guards `/finance/fixed-assets/register`, `/categories`, `/runs` and `/register/{id}`. The first three share one page adapter mirroring Bilimbi `FixedAssets.Web.ListPage`, with search and URL pagination (`page_size` 25, 50 or 100). Register uses the shared period filter and shows the dated report's ledger reconciliation. “Keep this report's recorded boundary” retains all four report pins; filtering, paging and period changes preserve explicitly supplied boundaries. Categories and runs retain the reference's full facade ordering and do not acquire extra status filters or sort controls.

Every list, including the asset's projected schedule, posted charges and events, uses `FlexTable` with `ListColumns` for column arrangement and compact rows. Register links and the asset page's announcement support workspace following. Timestamps use the company/browser/UTC display policy.

The shared Record form registers a manual-entry or posted-bill debit, creates/revises category defaults, or records a monthly run. Runs show skipped acquisitions individually and keep the reference's warning after an otherwise successful run. The asset page shows its frozen assumptions, acquisition provenance, recorded actor, projected schedule, posted charges and correction history, then disposal, void and reversal forms. Void waits for the shared `ConfirmDialog`; Cancel writes nothing. The form disappears permanently after a successful void. The facade decides all lifecycle and accounting refusals.

Each write event checks its current capability and the facade checks again inside its audited write. A refusal preserves the submitted draft and presents field errors; missing permissions show the reference's withheld-control sentence. `record_asset` is the submit event because the Platform reserves `save` for Company inline edits.

On a third fresh disposable database, run the NX3 page proof:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture --bin fixed-assets-ui
```

It drives the actual mounted Page/LiveState adapters against facade records: create/revise category defaults, search, page and page size, acquisition and field refusals, escaping and workspace markers, run skips, disposal/reversal, void confirmation/cancel, charge correction history, grants revoked after mount, withheld controls and column/compact controls. Add `serve` with matching `APP_ORIGIN` and `BIND_ADDR` to keep that disposable app available for browser proof; the fixture manager can write and contractor can view.

`evidence_page_on` is the same authorized evidence contract on a caller-supplied PostgreSQL client or transaction. Use it for every page of one repeatable-read export (Reports auditor pack); the pooled `evidence_page` wrapper is a live read per page. Schema helpers remain private.
