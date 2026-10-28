# Receivables

Company-scoped invoice and credit-note drafts, issuance, fully allocated receipts, corrections and reproducible reports, ported from `Bilimbi.Finance.Receivables` at `4abfaf829321f8540581d833752a086b21b45ba9`. Resolve `Receivables` at `finance/receivables.receivables`. Schema, SQL and validation are private; consumers use this facade.

## NR1 surface

| Methods | Capability | Contract |
| --- | --- | --- |
| `get_setup`, `list_setup_changes` | `finance.receivables.view` | Setup or `NotConfigured`; history in ascending ID order, exclusive `Page.after`, limit 1–500 (default 100). |
| `configure` | `finance.receivables.setup.manage` + ledger view | Active company asset `receivable_account_id` (not one that recorded receipts were paid into, unless it is already the saved account) and income `income_account_id`; one immutable history row per changed account with actor, impersonator and database-stamped UTC time. No history for a no-op. |
| `list_series`, `list_tax_codes`, `list_customers`, `get_customer` | `finance.receivables.view` | Code-ordered company records, including inactive records. A customer carries its linked Core company (`CompanyFacts`: name and status for every reader, tax ID, registration number, email and phone under `admin.company.view`, a restricted fact named in `restricted`). |
| `create_series`, `update_series` | `finance.receivables.setup.manage` | Kinds `invoice`, `credit_note`, `payment`; unique code and prefix per company; width 1–12. Updates change only name and active. |
| `create_tax_code`, `update_tax_code` | `finance.receivables.setup.manage` + ledger view | Percentage 0 ≤ rate < 1000, at most four meaningful decimal places; active liability account. Code and rate are immutable. An unchanged account is not revalidated on rename/deactivation. |
| `create_customer`, `update_customer`, `link_customer` | `finance.receivables.customers.manage` | A customer is a Finance record (`code`, immutable; `payment_terms_days` 0–3650; `active`) linked to a Core company by `core_company_id` (see [Customers and Core companies](#customers-and-core-companies)). Update changes terms and availability; a customer recorded before linking may still change its own party fields until `link_customer` links it, once, for good. |
| `create_draft`, `update_draft`, `discard_draft` | `finance.receivables.documents.manage` | Create/update also require ledger view, an open book and configured setup. Discard retains lines and totals and records its actor/time. |
| `get_document`, `list_documents` | `finance.receivables.view` | Header, lines and tax snapshots from one SQL snapshot; ascending IDs, `DocumentQuery` filters kind/status/customer and `Page` bounds as above. |

Every call takes the sealed scope and target company. Writes authorize before and inside the audited `MutationBoundary`, hold Core Company's writable row lock, and audit every affected header, history and child row. Ledger context reads use `Ledger::get_book_on` / `list_accounts_on` on that same transaction, so a writer never acquires a second connection while holding the company lock. An archived company refuses writes. Missing records and business refusals are `ReceivableError` variants; invalid fields retain Bilimbi's words, with one-based line positions for line errors.

Attributes are JSON objects with Bilimbi's field names. Partial updates preserve omitted fields and ignore unknown/immutable fields even if their supplied values are malformed. Blank editable fields reset to their schema defaults (active=true, terms=0, width=1); explicit null remains distinct. Decimal inputs accept finite scientific notation and reject underscores, whitespace and nonfinite values like Ecto. String fields are trimmed except series prefixes. All output types are plain records; amounts are `Money`, quantity/price/rate are exact `Decimal`, dates are ISO date strings, and timestamp strings represent database-stamped UTC.

## Customers and Core companies

A customer is a Finance record linked to a Core company (BelimbingApp/bilibi-dom-finance#75). The record keeps what is Finance's: the code, the payment terms and the Finance availability flag (`active`). The business is the company's: its display name, status, tax ID, registration number, contact details and billing address are read from Core on every read and never copied, so a change in Companies shows here at once. The link is a Base Records reference (`record_kind` `company`, `record_id`) stored on the row (`migrations/003-core-company.sql`); `Records::check` proves it on the write's own connection, so a company the actor may not view, a removed company or another tenant's is refused in Base Records' words, and the database keeps the link once set.

- **Reads.** `companies.rs` uses the ledger’s shared `CompanyLinks` reader on the connection the caller holds: `PartyProofs::organisation_name` names it and gives its status on any connection (a reviewed write's preview too), `bilibi_company::detail` adds the facts on a full connection under the reader's `admin.company.view`, masking what an administrator restricted for them (`CompanyFacts::restricted`). A reader without Companies view sees the name and status only (`readable: false`).
- **Invoicing policy.** A new invoice (draft create or update, issue) needs an invoiceable customer: linked, active for Finance, and its company `active` in Core. A `pending` company has not been activated yet and a `suspended` or `archived` one is not open for new business; each is refused on `customer_id` in its own words. Credit notes, receipts and reversals stay possible whatever the status or the link, so history settles and corrects as before.
- **Frozen party.** Issue freezes the company's name in the preview and its tax ID, registration number, email and phone at commit, on the transaction's full connection, under the issuer's own Companies view: without it the issue is refused, and a fact restricted for the issuer refuses too (a mask is never frozen). The billing address is not frozen yet: Core Address offers no read on the issuing connection, so the invoice page shows the current billing address from Companies and the `party_line*` columns stay empty for a linked customer (the address it was captured with before linking is not copied) until the Platform adds that read (BelimbingApp/bilibi#231, `TODO(platform)` in `companies.rs`).
- **Before linking.** Customers recorded before this migration keep the party fields they were captured with and read with `company: null`. They are listed (filter "Not linked to a company"), their history stays readable and settleable, and they cannot start a new invoice until `customer.link` links them to the company they are. The link is permanent (`finance_receivables_keep_link`); their own party fields stay on the row as history.
- **Relationship.** The pages record the customer relationship from the current company through Core's own reviewed contract `core.company.relationship.assign` (role code `customer`) right after the Finance write, and show whether Companies records it. Company exports neither a by-code role read nor a current-relationship read on a caller's connection, so Receivables does not compose this in its own transaction nor require it at issue yet (BelimbingApp/bilibi#231); an agent records it the same way after `customer.create`.
- **Permissions.** Reading a customer's company facts, linking and issuing need `admin.company.view`; finding one in the Add customer dialog needs `admin.company.list`; creating one there `admin.company.create`; recording the relationship `admin.company.update` with reach over the current company. Grant these to the roles that manage customers and issue invoices.

## Draft rules

- Required header: `kind`, `series_id`, `customer_id`. Optional: `issue_date`, `due_date`, `description`, `lines`. Credit notes additionally require `credited_document_id` naming an issued invoice of the same customer. An invoice's customer must be invoiceable (linked, active for Finance, its company active in Core) and all selected series active; a credit note may name an inactive or unlinked customer. Kind never changes.
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
| `issue_document(scope, company, id)` | `finance.receivables.documents.issue` + ledger view/post (+ `admin.company.view` for a linked customer) | Issues the saved draft without rebuilding lines or tax snapshots. Freezes the party (a linked customer's company name, tax ID, registration number, email and phone as Core holds them, read under the issuer's own Companies view and refused if a fact is restricted for them; a customer recorded before linking freezes its own party fields) and a missing invoice due date from current payment terms. Invoice control comes from current setup; credit control comes from its original invoice. Numbers by series prefix/width, posts with owner `finance/receivables` / `document:<id>`, and finalizes in the same audited transaction. |
| `record_payment(scope, company, attrs)` | `finance.receivables.payments.post` + ledger view/post | Requires setup, payment series, customer, payment date, amount, active asset deposit account (never the setup's receivable account or the saved receivable account of an allocated invoice) and allocations. Each `{document_id, amount}` names a distinct issued invoice of that customer dated no later than the payment; positive allocations must sum exactly to the payment. An array or singleton allocation object is accepted. No unallocated receipts or subsequent reallocation. |
| `reverse_payment(scope, company, id, attrs)` | `finance.receivables.payments.post` + ledger post | Required `reversal_date` no earlier than payment date and trimmed `reason` (1–500 characters). Corrects through `Ledger::reverse_owned_entry` with the original `payment:<id>` owner; returns payment with nested reversal. Original rows and allocations never change. Neither setup nor ledger view is required. |
| `get_payment`, `list_payments` | `finance.receivables.view` | Allocations in ID order and optional nested reversal. `PaymentQuery` supplies customer and `Page` bounds. |
| `list_issued_documents(scope, company, limit, after_entry_number)` | `finance.receivables.view` | Issued documents in posting order, exclusive entry-number cursor ≥ 0, limit 1–500 (default 100). |

Invoices require invoiceable customers (linked, active for Finance, the company active in Core), active series and saved active income/tax accounts, at least one line, an issue date and positive total. Credit notes cannot predate or exceed the original invoice's current outstanding. Credits and payments settle each invoice's saved asset control even after setup changes or control deactivation; new deposits still require an active asset account other than the current receivable account and the allocated invoices' receivable accounts. A customer may be inactive when paying or receiving a credit. Capacity checks use all recorded settlements, including future-dated ones. Only reports apply historical date bounds.

Every receipt posts a debit to its deposit and credits grouped by the original invoices' controls, owned by `payment:<id>`. Its table ID is reserved before posting; rollback may leave an identity gap, but never consumes a series number or leaves an entry without its receipt. Ledger period refusals propagate without changing dates. Every record, child, entry and audit capture commits or rolls back together.

## Reproducible figures

All reads require Receivables view; aging additionally reads the book through ledger view, and tie-out needs ledger view. Reads remain available on archived companies. Date and entry bounds apply independently to invoices, credits, receipts and reversals. Ordinary invoice `outstanding` includes all recorded events; historical reports do not use that unbounded figure.

- `aged_receivables(scope, company, as_of, &AgingOptions)` returns customer/document rows, buckets and totals. Default cutoffs `[30,60,90]` must be nonempty, strictly increasing, within 1–36500; due today is bucket zero. The default pin is the greatest Receivables document/payment/reversal/write-off entry number. Frozen party names accompany current customer names/codes (a linked customer's current name is its company's). The aging page names each customer by that current name and adds "(invoiced as …)" on an invoice issued under a different name. A past report is reproduced from its stored export, not by recomputing it after a rename.
- `tax_totals(scope, company, &TaxOptions)` sums saved tax lines by kind, prior-period credit flag, code, rate and account. Optional inclusive `from`/`to` and nonnegative `through_entry_number`; credit amounts stay positive. `credits_prior_period` means the credited invoice predates `from`.
- `received_tax_totals(scope, company, &ReceivedTaxOptions)` requires inclusive dates; unpaid months default to 12, range 1–120. Tax due day is issue date + calendar months + one day, not commercial due date. Pre-due allocations and reversals give signed `received` shares; outstanding on due-minus-one gives `unpaid`; credits on/after due give positive `credited_after_due`. Shares remain unrounded `Decimal`, grouped with distinct document counts and retained zero-net groups. Reporting rounding belongs to the consumer.
- `tie_out_control_accounts(scope, company, as_of, &BalancePins)` reads ledger owner totals first and pins the subledger to that watermark and its opening-batch boundary. Includes every historical issued control plus current setup, even inactive or fully settled controls. Shows all-owner balance, owned balance, other postings and subledger-minus-ledger difference per account; `tied_out` requires every difference to be zero, not merely their sum.

Tax reports return rows, not a captured watermark: pass the first report's pin for a reproducible multi-report reading. Aggregate amounts are `Decimal` because a sum of valid rows can exceed `Money`'s per-row storage bound. There is no `legacy` field: Laravel adoption is deliberately absent.

## Pages

Menu: Finance → Receivables, for people with `finance.receivables.view`. Controls the reader may not use are hidden. Every page reads and writes through `base/agent_api.pages` (`PageContracts`) and declares each write event in its `PageWrites`; no page touches the facade or a table. The lists read the contracts' cursors (up to 2,000 records) and filter, sort and page them on the server.

| Page | Path | What it does |
| --- | --- | --- |
| Customers | `/finance/receivables/customers` | Search and filter (including customers not yet linked), each with its company's status in Core; Add customer dialog: find the company in Companies (shared `RecordPicker`) or create it there (`core.company.create` with its duplicate review: use a likely match, or create anyway with a reason), then `customer.create`, then the customer relationship from the current company (`core.company.relationship.assign`). |
| Customer | `/finance/receivables/customers/{id}` | Payment terms and availability edited in place (`customer.update`); the company read from Companies (name as a record pill, status, tax ID, registration number, email, phone, the current billing address from `core.address.company.list`) and whether the current company records the relationship (`core.company.relationship.list`, with Record relationship); a customer recorded before linking edits its own details and address in place and links through Link to company (`customer.link`); the customer's invoices, credit notes and receipts. |
| Invoices | `/finance/receivables/invoices` | Invoices and credit notes by type, status (draft, issued, unpaid, discarded) and customer; New invoice and New credit note dialogs (`draft.create`). |
| Invoice | `/finance/receivables/invoices/{id}` | Dates and description edited in place and lines in a dialog while a draft (`draft.update`); Issue and Discard after a confirmation (`document.issue`, `draft.discard`); party, lines, tax, totals, posting status, receipts and credits applied, source documents. |
| Receipts | `/finance/receivables/receipts` | Record receipt dialog: the customer's open invoices with an amount each, applied in full, deposited to an active asset account other than the receivable account (`payment.record`). |
| Receipt | `/finance/receivables/receipts/{id}` | Facts, the invoices it settles, source documents, Reverse receipt dialog (`payment.reverse`). |
| Aging | `/finance/receivables/aging` | Who owes what by days late as of a date (one date, so not the shared period filter), read at one posting number (`aging.get`). |
| Settings | `/finance/receivables/settings` | For the company named under the title: the default accounts chosen in place (`setup.configure`; before the setup exists the first choice waits on the page until the second, then both are saved in one call; changing an existing receivable account asks for confirmation first and applies to new invoices only, and an asset account that recorded receipts were paid into is not offered, and `setup.configure` refuses it as a new receivable account), their change history folded under History, the numbering series and tax codes with name, status and tax account edited in their rows (`series.update`, `tax_code.update`) and Add dialogs for new ones (`series.create`, `tax_code.create`), and what is still missing before the first invoice (default accounts, an active series for invoices, credit notes and receipts; tax codes are optional). |

`finance.receivables.aging.get` is the one contract these pages added: the aged figures had a facade method (`aged_receivables`) and no operation. It takes `as_of` and optional `bucket_days`, needs receivables view and ledger view, and returns the report with the posting number it was read at.

## Source documents and evidence

Contributed sources are `finance/receivables/document` (Receivables document) and `finance/receivables/payment` (Payment received). They authorize and read on the supplied connection; consumers attach only through `Attachments`. Labels use document/payment numbers, not references. Discarded documents and payments are final: files can be added, never replaced or removed.

Bilibi's source contract requires a date where Bilimbi permits nil. An undated draft or discarded document therefore uses its stable UTC creation date for retention only; it never acquires a fabricated issue date. Issue calls `refresh_retention` inside its own transaction. The issue response alone carries `retention_refresh`: `queued`, `none`, or `error: unauthorized` if its issuer lacks source view. That pre-mutation authorization refusal does not add a view prerequisite to issuance. Unlike Bilimbi's best-effort refresh, other refresh failures roll back issuance because a failed queue/audit write can poison Bilibi's mutation transaction. Discard keeps the reference's existing behavior and does not refresh retention.

`evidence_datasets()` lists fifteen frozen datasets: opening_batch, opening_item, allocation, customer (its field list ends with the `record_kind` and `record_id` the link stores), document, document_line, payment, payment_reversal, series, setup, setup_change, tax_code, tax_line, write_off and write_off_event. `evidence_page(scope, company, dataset, limit, after)` requires view, returns explicit fields, exact-text dates/timestamps/decimals, rows and next ID; default limit 500, allowed 1–500, cursor ≥ 0. Pages are live reads, not one export snapshot. For reference parity the `document_line` field list retains its duplicate `quantity` column. No schema introspection or sibling-table access is exposed.

## Running proof

From the mounted Platform root, on an empty disposable PostgreSQL 18 database:

```sh
DATABASE_URL=postgres://…/disposable BILIBI_SETTINGS_KEY=$(openssl rand -hex 32) \
  cargo run -p bilibi-finance-walkthrough-fixture
```

The disposable encryption key is required by the shared walkthrough's e-invoice section, not by Receivables. Never replace a production encryption key with this specimen command.

The NR1 section asserts setup history/no-op behavior, identity and uniqueness, address normalization, schema-default resets, scientific notation, exact large JSON decimals, precision and overflow, per-line half-up tax, empty/replace/keep/clear/discard semantics, invalid replacement and audit-capture failure rollback, inactive records, paging, tenant/capability refusals (including a direct deny applied to an already-sealed scope), concurrent duplicate creation, archived-company refusal, database guards and complete replacement audit rows under one trace. It precedes the existing ledger/attachments/periods/controls proof and restores shared setup/account state. The mounted release gate remains the Platform's `scripts/domain-ci.sh`, without unit tests.

The NR2 section runs last and proves issue-time snapshots, numbering races, revoked grants, issue without source view, singleton/multi-control receipts, late audit-capture rollback, closed-period refusals, inactive-control settlement, owned correction, dated and pinned aging/tax figures, calendar-month tax boundaries and unrounded fractions, competing settlements/reversals, other-owner tie-out differences, both attachment sources, all fifteen evidence datasets, row-by-row audit capture and database immutability guards. Its amounts and dates are fixture data only, never production defaults.

## Reviewed agent operations (H2)

Discover `finance.receivables.*` through the shared `/api/v1` or `/mcp` edge and read the contributed `finance.receivables` guide ([guide.md](guide.md)). All inputs name `company_id`; identity and tenant come from the credential. The nineteen safe writes are `setup.configure`, `series.create/update`, `tax_code.create/update`, `customer.create/update/link`, `draft.create/update/discard`, `document.issue`, `payment.record`, `payment.reverse`, `write_off.submit/approve/reject/reverse` and `opening.import`. The thirteen reads are `setup.get/changes`, `series.list`, `tax_code.list`, `customer.get/list`, `document.get/list`, `payment.get/list`, `aging.get`, `write_off.accounts` and `opening.context`.

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

## Bad-debt journals

An overdue invoice can be written off through an invoice-linked journal, approved by a second person with the ledger checking identity overlap. **A write-off records an accounting decision; it does not establish tax relief.** The original invoice, revenue, tax snapshots and receipts remain intact.

For example, an invoice for 108 has a receipt of 54. Propose a write-off of the remaining 54 from its invoice page, naming the journal date, allowance or expense account, reason and collection evidence reference. Keep the assessment and collection records in the invoice's Source documents panel. Proposal posts nothing. Approval debits the selected allowance asset or bad-debt expense and credits the invoice's original receivables control in one audited transaction. This is a Receivables-owned journal entry, separate from Ledger's general manual-journal queue; review it on the invoice.

The company chooses its accounting policy and accounts. The software accepts an active asset allowance or expense account, excluding every current and historical receivables control and every account receipts have been paid into (a bank or cash account), so a write-off never debits cash. It does not estimate expected credit losses, assess whether recovery is reasonably expected, verify an evidence reference, or determine an allowance balance. The reviewer makes those assessments. A journal date must follow the invoice due date and cannot precede recorded receipts, credit notes, receipt reversals or earlier write-off decisions. There is no automatic write-off based on months overdue.

### Recovery and correction

Reverse an approved write-off with a reason and an open-period date on or after the write-off. This posts the owned mirror journal and restores the invoice's balance atomically. Record the recovery through the normal receipt operation, allocated to that original invoice. A partial recovery restores the entire written-off amount first; reassess and propose a new write-off for any remaining irrecoverable balance. If the company's policy requires a separate recovery-income presentation, its accountant records the appropriate reclassification; the receipt is not a new sale. An unapproved proposal can be rejected with a reason, then replaced by a new proposal.

Approval, rejection and reversal use Ledger's identity-overlap review and `Ledger::check_approval`, including impersonation. This workflow requires a different person from the proposer even when the book's general separation setting is off; an agent counts as the person it acts for. No same-person override is offered. Only Receivables can reverse its owned entry. A closed period, archived company, withdrawn grant, changed outstanding amount, inactive offset or duplicate decision refuses the write. No partial posting survives a refusal.

### Reports and history

Aging and control-account tie-out deduct approved write-offs and restore reversals at their effective dates and entry-number pins. Fully written-off invoices leave open aging but remain on the invoice list with their original total, zero outstanding and complete Bad-debt journals history. Ordinary `Document.outstanding` includes all recorded write-off decisions, as it includes all recorded receipts. The History disclosure retains proposal and decision reasons, evidence reference, users, impersonators, timestamps and entry links. Tax totals and the twelve-month unpaid-tax schedule use gross commercial amounts before accounting write-offs: a write-off is neither a receipt nor a tax credit.

Migration `finance-receivables-002-write-offs` adds two append-only tables and extends settlement capacity to include write-offs. Both tables have evidence datasets (`write_off`, `write_off_event`) with explicit fields. The public document read includes `write_offs`; no consumer reads these tables directly.

### Contracts and permissions

Every contract requires `company_id`. Reads and writes are available through the same UI, HTTP API and MCP contracts. The agent review fence binds invoice capacity, original control, account eligibility, posting controls and approval facts before committing on the shared transaction.

| Contract / facade | Additional inputs | Permission |
| --- | --- | --- |
| `write_off.accounts` | none; returns eligible `accounts` | Receivables view + Ledger view |
| `write_off.submit` / `submit_write_off` | `document_id`, `entry_date`, `amount` as decimal text, `offset_account_id`, `reason`, `evidence` | `finance.receivables.write-offs.submit`, Ledger view/post |
| `write_off.approve` / `approve_write_off` | `write_off_id`, review `reason` | `finance.receivables.write-offs.approve`, Ledger view/post |
| `write_off.reject` / `reject_write_off` | `write_off_id`, review `reason` | `finance.receivables.write-offs.approve`, Ledger view |
| `write_off.reverse` / `reverse_write_off` | `write_off_id`, `reversal_date`, `reason` | `finance.receivables.write-offs.approve`, Ledger view/post |

Contract keys above have the `finance.receivables.` prefix. Amounts obey the book's minor units; reasons and evidence references are required, trimmed and limited to 500 characters. Each facade method takes scope, company and the same JSON attributes. Grant these permissions explicitly when adopting the workflow; existing user-defined roles acquire no write-off authority automatically.

### Supported boundary and proof

Run `cargo run -p bilibi-finance-walkthrough-fixture --bin bad-debt` on a fresh disposable database, mounted in the pinned Platform. Set `APP_ORIGIN` and `BIND_ADDR` and add `serve` to keep its records for browser verification. Use `BILIBI_AGENT_PROOF=disposable` with `serve-existing` to serve those installed fixture records again without reseeding. To include actual collection evidence, set `BAD_DEBT_STORAGE` to a task-owned folder created with mode `0700`. It demonstrates unpaid and part-paid invoices, the existing twelve-month tax boundary, direct and allowance write-offs, independent approval, rejection, reversal, partial recovery, pinned aging, control tie-out, source evidence references and audited exports. A specimen external approval demonstrates SST's existing manual 13(d) adjustment and a separate recovery-repayment adjustment; it is not evidence of an actual refund approval or eligibility. A return adjustment changes the return figures, not ledger postings or tax-payment records. The proof reconciles original invoice tax to received and twelve-month unpaid tax separately from that adjustment.

For service-tax relief on a partly paid taxable service, Act 807 section 35(2) specifies tax paid less `A × C / B`: A is payment received for the taxable services, B is their value plus service tax payable, and C is service tax payable. Tax paid and payable need not be equal. For illustration only, a service worth RM100 with RM8 tax payable and paid, and RM54 received, gives RM8 − (RM54 × RM8 / RM108) = RM4 before eligibility and approval. Section 36 separately applies a formula to later receipts. The twelve-month tax-due backstop is not a bad-debt eligibility waiting period. The company must assess the claim, payment and evidence; the fixture does not prove eligibility or DG approval. See [Act 807](https://lom.agc.gov.my/ilims/upload/portal/akta/outputaktap/3465678_BI/Act%20807%20(Online%202026).pdf) and [Service Tax Regulations 2018](https://mysst.customs.gov.my/wp-content/uploads/2025/03/Service-Tax-Regulations-2018.pdf).

Income-tax treatment is separate. Under [LHDN Public Ruling 4/2019](https://www.hasil.gov.my/wp-content/uploads/PR_04_2019.pdf), a qualifying trade debt must have been included in gross income in a prior basis period; reasonable recovery steps and evidence matter. A specific doubtful-debt provision requires an individual debtor assessment and evidence; a general provision or non-trade debt is not deductible. Recovery after an allowed deduction is gross income in the period received. A book write-off or SST adjustment does not establish a deduction.

Not implemented: collection/dispute case management, impairment measurement, an SST refund application/approval register or automatic repayment calculation, tax filing, income-tax computations, or a combined multi-company book. Malaysian SST and income-tax treatment needs accountant confirmation for each company's facts; the dated practitioner investigation is in [issue #60](https://github.com/BelimbingApp/bilibi-dom-finance/issues/60), and remaining questions are tracked in [issue #96](https://github.com/BelimbingApp/bilibi-dom-finance/issues/96).

## Opening invoices at accounting system conversion

Import a complete opening invoice register after the source trial balance has been approved, before issuing ordinary invoices. The conversion date is the first day Bilibi owns processing; source invoice dates must precede it. Configure the receivables control account, create the customers and a dedicated invoice numbering series, then open **Receivables → Opening invoices** (`/finance/receivables/opening`). The current company comes from the session choice.

Paste UTF-8 CSV with this exact header:

```csv
source_id,customer_code,document_reference,document_date,due_date,original_total,outstanding,control_account_code
```

Use original gross totals and only the remaining amount at cut-off. For example, a 5,000 invoice with 2,000 already settled imports an outstanding balance of 3,000. `customer_code` links to an active customer; `control_account_code` must identify the configured active control account. Original dates, references, totals and the amount settled before conversion remain source evidence. The internal invoice reference uses the selected series; the record page also shows the source reference and original total. These records represent opening residuals, without a new taxable supply or tax posting.

Choose **Preview import**, reconcile the complete register to the approved opening control balance, then **Approve import**. A mismatch refuses the whole import before any records are written. The approver must be independent of the opening journal's source preparer, including impersonation, unless the existing duty-separation controls permit a recorded exception. The exact CSV and approval identity are retained in the audited transaction. The source opening entry cannot subsequently be reversed while these records depend on it.

An agent uses `finance.receivables.opening.context` to read the approved entry, book, required CSV header and existing batch, and `finance.receivables.opening.import` through the ordinary plan/commit review. Input is `company_id`, `source_reference`, `currency_code`, `series_id`, `csv`, and optional `override_reason`. The facade is `Receivables::import_opening_invoices`; an agent commit joins its handed transaction through the private `import_opening_on`. No operation can bypass missing setup, reconciliation or duty separation.

Record receipts and reverse them through the normal receipt operations. Opening invoices keep the saved control account after setup changes. Aging uses original due dates from conversion onward; a date before conversion excludes them, and settlement before conversion is refused. Import creates no entry, income, tax lines or synthetic historical payment allocations. Ordinary tax and received-tax reports therefore contain no source-period tax detail for these residuals. Control tie-outs compare the resulting residuals with actual ledger balances; the source journal keeps its original manual owner, with separate opening-recognition evidence rather than an ownership rewrite.

Each company can import one complete register of 1–500 positive residual invoices, at most 128 KiB. Retry with the same source reference, series and byte-identical CSV to read the retained batch, including after settlement. Changed retries are refused. Preview and cancel are reversible; **correction or replacement of a committed opening register is not yet supported**. Opening credits, multiple historical control accounts, foreign-currency translation and larger staged imports require follow-up work. Preserve those source schedules and resolve their treatment before importing; do not force a tie-out with a suspense amount or ordinary sales invoices.

Evidence datasets `opening_batch` and `opening_item` retain the exact source export and each imported invoice's source facts. The accountant remains responsible for source completeness, mapping, cut-off, source-period tax, YTD/comparative reporting and reconciliation sign-off.

Aging and control-account reports return `through_entry_number` and `through_opening_batch_id`. Retain both to reproduce a report across an entryless opening import; `report.pins()` supplies `BalancePins` for a tie-out. `AgingOptions` accepts both boundaries. A zero opening-batch pin excludes opening records.
