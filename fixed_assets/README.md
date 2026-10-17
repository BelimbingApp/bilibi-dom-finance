# Fixed assets acquisition

`finance/fixed_assets` mirrors Bilimbi's `Bilimbi.Finance.FixedAssets` at `4abfaf829321f8540581d833752a086b21b45ba9`. NX1 owns immutable categories, complete category revisions and acquired assets, with projected book depreciation. Runs, corrections, register reports and their pages belong to NX2/NX3. This slice contributes no routes, menu items, agent operations, CSS or JavaScript.

## Facade

Resolve `FixedAssets` at `finance/fixed_assets.fixed_assets`. Every data call takes a sealed `Scope` and company ID and checks company reach and capability. Writes use `Ledger::write`: capability checks before and inside the audited `MutationBoundary`, writable-company row lock, then every affected record captured as a `Change`. Actors and impersonators come only from the scope. Reads of source records use the providers' authorized `_on` facade methods on the caller's connection, so no write waits for a second pool slot while holding the company lock.

| Call | Capability | Result |
| --- | --- | --- |
| `create_category(scope, company, name, RevisionInput)` | `finance.fixed-assets.finance-asset-configure` | Category and first revision atomically; returns the revision with its category name. |
| `revise_category(scope, company, category_id, RevisionInput)` | same | Complete new revision; never a patch or rename. |
| `list_categories(scope, company)` | `finance.fixed-assets.view` | Name-ordered categories, latest revision and full ID-ordered history. |
| `acquire_from_bill(scope, company, bill_id, line_number, AcquisitionInput)` | `finance.fixed-assets.finance-asset-acquire` | Asset registered from a posted supplier bill's net debit line. Also requires Payables and Ledger view. |
| `acquire_from_entry(scope, company, entry_id, line_number, AcquisitionInput)` | same | Asset registered from a live original debit. Also requires Ledger view. |
| `get_asset(scope, company, id)` | `finance.fixed-assets.view` | Asset, original category revision and complete projected schedule. Also requires Ledger view to read book precision. |
| `evidence_page(scope, company, dataset, limit, after)` | `finance.fixed-assets.view` | Explicit frozen fields, exact decimal text, UTC timestamps; ID cursor and limit 1–500. Datasets: `asset`, `category`, `category_revision`. |

All seven reference capability keys and six `finance-asset-*` verbs are declared and granted to `tenant_owner`; depreciation, reversal, disposal and void verbs have no callable workflow until NX2.

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

## Storage and refusals

Migration `finance-fixed-assets-001-acquisitions` creates the three NX1 tables. PostgreSQL assigns company-local asset numbers under its advisory transaction lock, stamps UTC recording time, enforces scoped composite references and unique source debits, and refuses UPDATE, DELETE and TRUNCATE permanently. No sibling SQL or cross-domain dependency is used.

Expected refusals are `FixedAssetsError` variants (`BillNotPosted`, `BillLineNotFound`, `EntryLineNotFound`, `NotAcquisitionDebit`, `AcquisitionReversed`, reach/archive/not-found) or `Invalid(Vec<FieldError>)`. Duplicate names and sources are field refusals decided before mutation; provider failures retain their owning error. Two deliberate corrections to reference defects: blank revision reasons are refused before SQL; SQL checks require a non-null reducing-balance rate and both bill-source fields together instead of allowing nullable `CHECK` expressions to pass.

## Running proof

From the mounted Platform root, on a fresh disposable database:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture --bin fixed-assets
```

The binary migrates and installs the disposable walkthrough fixture, then proves category histories, defaults frozen after revision, book precision, source eligibility, posted-bill line provenance and ownership, straight-line ties, reducing balance with residual/final true-up, caps and zero rows, leap-month labels, historical inactive accounts, concurrent duplicates, reversal eligibility, revoked provider/owner grants, company scope, archive refusal, paged evidence, audited rows and PostgreSQL retention/uniqueness/check/composite-key refusals. It adds no production seeds and no unit tests. The repository gate remains the Platform's `scripts/domain-ci.sh`.
