# Receivables

Company-scoped invoice and credit-note drafts, issuance, fully allocated receipts, corrections and reproducible reports, ported from `Bilimbi.Finance.Receivables` at `4abfaf829321f8540581d833752a086b21b45ba9`. Resolve `Receivables` at `finance/receivables.receivables`. Schema, SQL and validation are private; consumers use this facade.

## NR1 surface

| Methods | Capability | Contract |
| --- | --- | --- |
| `get_setup`, `list_setup_changes` | `finance.receivables.view` | Setup or `NotConfigured`; history in ascending ID order, exclusive `Page.after`, limit 1–500 (default 100). |
| `configure` | `finance.receivables.setup.manage` + ledger view | Active company asset `receivable_account_id` and income `income_account_id`; one immutable history row per changed account with actor, impersonator and database-stamped UTC time. No history for a no-op. |
| `list_series`, `list_tax_codes`, `list_customers`, `get_customer` | `finance.receivables.view` | Code-ordered company records, including inactive records. |
| `create_series`, `update_series` | `finance.receivables.setup.manage` | Kinds `invoice`, `credit_note`, `payment`; unique code and prefix per company; width 1–12. Updates change only name and active. |
| `create_tax_code`, `update_tax_code` | `finance.receivables.setup.manage` + ledger view | Percentage 0 ≤ rate < 1000, at most four meaningful decimal places; active liability account. Code and rate are immutable. An unchanged account is not revalidated on rename/deactivation. |
| `create_customer`, `update_customer` | `finance.receivables.customers.manage` | Flat free-text party/address fields, optional uppercase two-letter `country_iso`, payment terms 0–3650 days; code immutable. No Core party/address reference or country catalog lookup. |
| `create_draft`, `update_draft`, `discard_draft` | `finance.receivables.documents.manage` | Create/update also require ledger view, an open book and configured setup. Discard retains lines and totals and records its actor/time. |
| `get_document`, `list_documents` | `finance.receivables.view` | Header, lines and tax snapshots from one SQL snapshot; ascending IDs, `DocumentQuery` filters kind/status/customer and `Page` bounds as above. |

Every call takes the sealed scope and target company. Writes authorize before and inside the audited `MutationBoundary`, hold Core Company's writable row lock, and audit every affected header, history and child row. Ledger context reads use `Ledger::get_book_on` / `list_accounts_on` on that same transaction, so a writer never acquires a second connection while holding the company lock. An archived company refuses writes. Missing records and business refusals are `ReceivableError` variants; invalid fields retain Bilimbi's words, with one-based line positions for line errors.

Attributes are JSON objects with Bilimbi's field names. Partial updates preserve omitted fields and ignore unknown/immutable fields even if their supplied values are malformed. Blank editable fields reset to their schema defaults (active=true, terms=0, width=1); explicit null remains distinct. Decimal inputs accept finite scientific notation and reject underscores, whitespace and nonfinite values like Ecto. String fields are trimmed except series prefixes. All output types are plain records; amounts are `Money`, quantity/price/rate are exact `Decimal`, dates are ISO date strings, and timestamp strings represent database-stamped UTC.

## Draft rules

- Required header: `kind`, `series_id`, `customer_id`. Optional: `issue_date`, `due_date`, `description`, `lines`. Credit notes additionally require `credited_document_id` naming an issued invoice of the same customer. Invoice customers and all selected series must be active; a credit note may name an inactive customer. Kind never changes.
- Drafts can have no dates, no lines and zero total. They consume no series number and post no ledger entry. Due date cannot precede issue date.
- Each line requires description, quantity > 0 and unit price ≥ 0; quantity and unit price allow four meaningful fractional places and must be less than 1E16. `account_id` defaults to the setup's income account, validated active and income-typed. `tax_code_id` is optional and must be active if supplied.
- Amount is quantity × price rounded half up to book minor units; tax is the rounded amount × percentage / 100, rounded again per line. Totals sum those rounded lines; tax groups snapshot code, rate and liability account. Arithmetic is checked, and monetary values/totals reaching 1E16 are refused before SQL. JSON decimal tokens never pass through binary floating point.
- Missing/null `lines` preserves existing children, IDs, totals and snapshots on update; `[]` clears both child sets and totals. A supplied array (or single line object) replaces both child sets atomically. Invalid replacement changes nothing.

## Schema and scope boundary

Migration `finance-receivables-001-receivables` is the fresh final reference schema: eleven tables, identity/append-only/draft guards, database numbering and settlement constraints. It deliberately includes the final payment tables/functions so future migrations need not rewrite the reference guards. No Laravel adoption or legacy entry claiming is ported. Composite keys keep this module's references inside one company; ledger account and entry IDs have no storage-level dependency on sibling tables.

Bilimbi's Receivables has no pages (`web: nil`). H2 exposes its facade-only actions through the shared reviewed agent API, and the pages below call those same contracts so a person without an agent does the same work (ADR 01). The module owns no CSS or JavaScript.

## Issuance and settlement

| Method | Capability | Contract |
| --- | --- | --- |
| `issue_document(scope, company, id)` | `finance.receivables.documents.issue` + ledger view/post | Issues the saved draft without rebuilding lines or tax snapshots. Freezes the current customer party and missing invoice due date from current payment terms. Invoice control comes from current setup; credit control comes from its original invoice. Numbers by series prefix/width, posts with owner `finance/receivables` / `document:<id>`, and finalizes in the same audited transaction. |
| `record_payment(scope, company, attrs)` | `finance.receivables.payments.post` + ledger view/post | Requires setup, payment series, customer, payment date, amount, active asset deposit account and allocations. Each `{document_id, amount}` names a distinct issued invoice of that customer dated no later than the payment; positive allocations must sum exactly to the payment. An array or singleton allocation object is accepted. No unallocated receipts or subsequent reallocation. |
| `reverse_payment(scope, company, id, attrs)` | `finance.receivables.payments.post` + ledger post | Required `reversal_date` no earlier than payment date and trimmed `reason` (1–500 characters). Corrects through `Ledger::reverse_owned_entry` with the original `payment:<id>` owner; returns payment with nested reversal. Original rows and allocations never change. Neither setup nor ledger view is required. |
| `get_payment`, `list_payments` | `finance.receivables.view` | Allocations in ID order and optional nested reversal. `PaymentQuery` supplies customer and `Page` bounds. |
| `list_issued_documents(scope, company, limit, after_entry_number)` | `finance.receivables.view` | Issued documents in posting order, exclusive entry-number cursor ≥ 0, limit 1–500 (default 100). |

Invoices require active customers/series and saved active income/tax accounts, at least one line, an issue date and positive total. Credit notes cannot predate or exceed the original invoice's current outstanding. Credits and payments settle each invoice's saved asset control even after setup changes or control deactivation; new deposits still require an active asset account. A customer may be inactive when paying or receiving a credit. Capacity checks use all recorded settlements, including future-dated ones. Only reports apply historical date bounds.

Every receipt posts a debit to its deposit and credits grouped by the original invoices' controls, owned by `payment:<id>`. Its table ID is reserved before posting; rollback may leave an identity gap, but never consumes a series number or leaves an entry without its receipt. Ledger period refusals propagate without changing dates. Every record, child, entry and audit capture commits or rolls back together.

## Reproducible figures

All reads require Receivables view; aging additionally reads the book through ledger view, and tie-out needs ledger view. Reads remain available on archived companies. Date and entry bounds apply independently to invoices, credits, receipts and reversals. Ordinary invoice `outstanding` includes all recorded events; historical reports do not use that unbounded figure.

- `aged_receivables(scope, company, as_of, &AgingOptions)` returns customer/document rows, buckets and totals. Default cutoffs `[30,60,90]` must be nonempty, strictly increasing, within 1–36500; due today is bucket zero. The default pin is the greatest Receivables document/payment/reversal entry number. Frozen party names accompany current customer names/codes.
- `tax_totals(scope, company, &TaxOptions)` sums saved tax lines by kind, prior-period credit flag, code, rate and account. Optional inclusive `from`/`to` and nonnegative `through_entry_number`; credit amounts stay positive. `credits_prior_period` means the credited invoice predates `from`.
- `received_tax_totals(scope, company, &ReceivedTaxOptions)` requires inclusive dates; unpaid months default to 12, range 1–120. Tax due day is issue date + calendar months + one day, not commercial due date. Pre-due allocations and reversals give signed `received` shares; outstanding on due-minus-one gives `unpaid`; credits on/after due give positive `credited_after_due`. Shares remain unrounded `Decimal`, grouped with distinct document counts and retained zero-net groups. Reporting rounding belongs to the consumer.
- `tie_out_control_accounts(scope, company, as_of, through)` reads ledger owner totals first and pins the subledger to that watermark. Includes every historical issued control plus current setup, even inactive or fully settled controls. Shows all-owner balance, owned balance, other postings and subledger-minus-ledger difference per account; `tied_out` requires every difference to be zero, not merely their sum.

Tax reports return rows, not a captured watermark: pass the first report's pin for a reproducible multi-report reading. Aggregate amounts are `Decimal` because a sum of valid rows can exceed `Money`'s per-row storage bound. There is no `legacy` field: Laravel adoption is deliberately absent.

## Pages

Menu: Finance → Receivables, for people with `finance.receivables.view`. Controls the reader may not use are hidden. Every page reads and writes through `base/agent_api.pages` (`PageContracts`) and declares each write event in its `PageWrites`; no page touches the facade or a table. The lists read the contracts' cursors (up to 2,000 records) and filter, sort and page them on the server.

| Page | Path | What it does |
| --- | --- | --- |
| Customers | `/finance/receivables/customers` | Search and filter, Add customer dialog (`customer.create`). |
| Customer | `/finance/receivables/customers/{id}` | Details and address edited in place (`customer.update`); the customer's invoices, credit notes and receipts. |
| Invoices | `/finance/receivables/invoices` | Invoices and credit notes by type, status (draft, issued, unpaid, discarded) and customer; New invoice and New credit note dialogs (`draft.create`). |
| Invoice | `/finance/receivables/invoices/{id}` | Dates and description edited in place and lines in a dialog while a draft (`draft.update`); Issue and Discard after a confirmation (`document.issue`, `draft.discard`); party, lines, tax, totals, posting status, receipts and credits applied, source documents. |
| Receipts | `/finance/receivables/receipts` | Record receipt dialog: the customer's open invoices with an amount each, applied in full (`payment.record`). |
| Receipt | `/finance/receivables/receipts/{id}` | Facts, the invoices it settles, source documents, Reverse receipt dialog (`payment.reverse`). |
| Aging | `/finance/receivables/aging` | Who owes what by days late as of a date, read at one posting number (`aging.get`). |
| Settings | `/finance/receivables/settings` | For the company named under the title: the default accounts chosen in place (`setup.configure`; before the setup exists the first choice waits on the page until the second, then both are saved in one call), their change history folded under History, the numbering series and tax codes with name, status and tax account edited in their rows (`series.update`, `tax_code.update`) and Add dialogs for new ones (`series.create`, `tax_code.create`), and what is still missing before the first invoice (default accounts, an active series for invoices, credit notes and receipts; tax codes are optional). |

`finance.receivables.aging.get` is the one contract these pages added: the aged figures had a facade method (`aged_receivables`) and no operation. It takes `as_of` and optional `bucket_days`, needs receivables view and ledger view, and returns the report with the posting number it was read at.

## Source documents and evidence

Contributed sources are `finance/receivables/document` (Receivables document) and `finance/receivables/payment` (Payment received). They authorize and read on the supplied connection; consumers attach only through `Attachments`. Labels use document/payment numbers, not references. Discarded documents and payments are final: files can be added, never replaced or removed.

Bilibi's source contract requires a date where Bilimbi permits nil. An undated draft or discarded document therefore uses its stable UTC creation date for retention only; it never acquires a fabricated issue date. Issue calls `refresh_retention` inside its own transaction. The issue response alone carries `retention_refresh`: `queued`, `none`, or `error: unauthorized` if its issuer lacks source view. That pre-mutation authorization refusal does not add a view prerequisite to issuance. Unlike Bilimbi's best-effort refresh, other refresh failures roll back issuance because a failed queue/audit write can poison Bilibi's mutation transaction. Discard keeps the reference's existing behavior and does not refresh retention.

`evidence_datasets()` lists eleven frozen datasets: allocation, customer, document, document_line, payment, payment_reversal, series, setup, setup_change, tax_code and tax_line. `evidence_page(scope, company, dataset, limit, after)` requires view, returns explicit fields, exact-text dates/timestamps/decimals, rows and next ID; default limit 500, allowed 1–500, cursor ≥ 0. Pages are live reads, not one export snapshot. For reference parity the `document_line` field list retains its duplicate `quantity` column. No schema introspection or sibling-table access is exposed.

## Running proof

From the mounted Platform root, on an empty disposable PostgreSQL 18 database:

```sh
DATABASE_URL=postgres://…/disposable BILIBI_SETTINGS_KEY=$(openssl rand -hex 32) \
  cargo run -p bilibi-finance-walkthrough-fixture
```

The disposable encryption key is required by the shared walkthrough's e-invoice section, not by Receivables. Never replace a production encryption key with this specimen command.

The NR1 section asserts setup history/no-op behavior, identity and uniqueness, address normalization, schema-default resets, scientific notation, exact large JSON decimals, precision and overflow, per-line half-up tax, empty/replace/keep/clear/discard semantics, invalid replacement and audit-capture failure rollback, inactive records, paging, tenant/capability refusals (including a direct deny applied to an already-sealed scope), concurrent duplicate creation, archived-company refusal, database guards and complete replacement audit rows under one trace. It precedes the existing ledger/attachments/periods/controls proof and restores shared setup/account state. The mounted release gate remains the Platform's `scripts/domain-ci.sh`, without unit tests.

The NR2 section runs last and proves issue-time snapshots, numbering races, revoked grants, issue without source view, singleton/multi-control receipts, late audit-capture rollback, closed-period refusals, inactive-control settlement, owned correction, dated and pinned aging/tax figures, calendar-month tax boundaries and unrounded fractions, competing settlements/reversals, other-owner tie-out differences, both attachment sources, all eleven evidence datasets, row-by-row audit capture and database immutability guards. Its amounts and dates are fixture data only, never production defaults.

## Reviewed agent operations (H2)

Discover `finance.receivables.*` through the shared `/api/v1` or `/mcp` edge and read the contributed `finance.receivables` guide ([guide.md](guide.md)). All inputs name `company_id`; identity and tenant come from the credential. The thirteen safe writes are `setup.configure`, `series.create/update`, `tax_code.create/update`, `customer.create/update`, `draft.create/update/discard`, `document.issue`, `payment.record` and `payment.reverse`. The eleven reads are `setup.get/changes`, `series.list`, `tax_code.list`, `customer.get/list`, `document.get/list`, `payment.get/list` and `aging.get`.

Writes reuse the same private facade preparation and transaction-taking methods as pooled callers. Preview validates on the handed `OperationClient`, without allocating IDs or touching business/audit/queue rows. Ledger's posting/reversal previews own accounting validation. Execution joins the plan's `MutationTransaction`; no nested commit or pooled side read escapes its rollback. The owner reauthorizes and locks the writable company at preview and execution. Required capabilities remain the facade's existing ones, including reversal without setup/ledger view and issuance without source view. Committed results carry `record` and `path`, the record's page or the settings page.

The pinned engine previews all steps before any execution. A separate transaction-local FIFO rechecks each Receivables review immediately before mutation. It binds normalized values, defaults, saved snapshots, original controls, outstanding amounts, exact business numbers and applicable posting controls. Dependent steps (including two issues in the same series or settlements sharing an invoice) require separate plans; a refusal rolls back every business, ledger, audit and queue row and leaves the code unused. Independent chart/calendar changes do not invalidate unrelated Receivables steps. Failed execution may leave identity gaps, never skipped business series numbers.

Wire decimals are strings; strict schemas refuse unknown/immutable fields, actor fields and explicit null. Omission preserves optional values. Empty text clears optional party text, description and draft dates; clearing due date restores issue-time terms derivation. Omitted lines preserve snapshots, arrays replace them and `[]` clears them. Lists have `page_size` 1–100, default 100. Code lists compare their exclusive `after_code` with the database's ordering collation; ID lists use exclusive `after` and finish on an empty page. Documents filter kind/status/customer, payments filter customer. Reports remain the separately documented facade surface.

### Running the HTTP/MCP proof

Use a new empty disposable PostgreSQL 18 database (ICU collation exercises code cursors). From the mounted Platform root:

```sh
export DATABASE_URL=postgres://…/empty_receivables_proof
export BILIBI_AGENT_PROOF=disposable
export BILIBI_AGENT_CREDENTIALS=/tmp/receivables-agent-credentials.json
export APP_ORIGIN=http://127.0.0.1:3100
cargo build -p bilibi-web -p bilibi-finance-walkthrough-fixture --bins
./target/debug/agent-operations
amp orb service start receivables-proof --port 3100 --command \
  'BIND_ADDR=127.0.0.1:3100 ./target/debug/bilibi-web run'
python3 apps/domains/finance/fixtures/walkthrough/receivables-operations.py
```

The bootstrap saves real resource-bound OAuth credentials in a new mode-0600 file; never print it or use a production database. `receivables-control` makes only fixture-scoped grant/archive changes through normal facades in a second session. The wire proof exercises the 23 pre-existing operations on HTTP and MCP (`aging.get` is proved through its page), exact half-up totals and posting sides, preserved/cleared snapshots and dates, controls after setup changes, original-owned reversal, repeated non-mutating previews, replay, strict schemas, capability revocation, tenant isolation, archived targets, ICU/ID paging, mixed H1/H2 independent batching, dependent-plan rollback and late audit-capture failure/recovery. Run the H1 wire walkthrough on a separate fresh database to check the shared Ledger seams. No unit tests are added.

`evidence_page_on` is the same authorized evidence contract on a caller-supplied PostgreSQL client or transaction. Use it for every page of one repeatable-read export (Reports auditor pack); the pooled `evidence_page` wrapper is a live read per page. Schema helpers remain private.
