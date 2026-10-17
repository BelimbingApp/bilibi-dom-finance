# Receivables

Company-scoped receivables setup and unnumbered drafts, ported from `Bilimbi.Finance.Receivables` at `4abfaf829321f8540581d833752a086b21b45ba9`. Resolve `Receivables` at `finance/receivables.receivables`. Schema, SQL and validation are private; consumers use this facade.

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

Attributes are JSON objects with Bilimbi's field names. Partial updates preserve omitted fields and ignore unknown/immutable fields even if their supplied values are malformed. String fields are trimmed except series prefixes. All output types are plain records; amounts are `Money`, quantity/price/rate are exact `Decimal`, dates are ISO date strings, and timestamp strings represent database-stamped UTC.

## Draft rules

- Required header: `kind`, `series_id`, `customer_id`. Optional: `issue_date`, `due_date`, `description`, `lines`. Credit notes additionally require `credited_document_id` naming an issued invoice of the same customer. Invoice customers and all selected series must be active; a credit note may name an inactive customer. Kind never changes.
- Drafts can have no dates, no lines and zero total. They consume no series number and post no ledger entry. Due date cannot precede issue date.
- Each line requires description, quantity > 0 and unit price ≥ 0; quantity and unit price allow four meaningful fractional places and must be less than 1E16. `account_id` defaults to the setup's income account, validated active and income-typed. `tax_code_id` is optional and must be active if supplied.
- Amount is quantity × price rounded half up to book minor units; tax is the rounded amount × percentage / 100, rounded again per line. Totals sum those rounded lines; tax groups snapshot code, rate and liability account. Arithmetic is checked, and monetary values/totals reaching 1E16 are refused before SQL. JSON decimal tokens never pass through binary floating point.
- Missing/null `lines` preserves existing children, IDs, totals and snapshots on update; `[]` clears both child sets and totals. A supplied array (or single line object) replaces both child sets atomically. Invalid replacement changes nothing.

## Schema and scope boundary

Migration `finance-receivables-001-receivables` is the fresh final reference schema: eleven tables, identity/append-only/draft guards, database numbering and settlement constraints. It deliberately includes the final payment tables/functions so future migrations need not rewrite the reference guards. No Laravel adoption or legacy entry claiming is ported. Composite keys keep this module's references inside one company; ledger account and entry IDs have no storage-level dependency on sibling tables.

NR2 owns issuing, numbering, party snapshots, owned ledger posting, payments/allocations/reversals, reports, attachment sources and evidence exports. Their schema and six capability keys exist now, but their write methods are not exposed. Receivables has no pages, menu, CSS, JavaScript or agent operations, matching Bilimbi's `web: nil`; no new UI is invented for this slice.

## Running proof

From the mounted Platform root, on an empty disposable PostgreSQL 18 database:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture
```

The NR1 section asserts setup history/no-op behavior, identity and uniqueness, address normalization, decimal precision and overflow, per-line half-up tax, empty/replace/keep/clear/discard semantics, invalid replacement rollback, inactive records, paging, tenant/capability refusals, concurrent duplicate creation, archived-company refusal, database guards and audit capture. It precedes the existing ledger/attachments/periods/controls proof and restores shared setup/account state. The mounted release gate remains the Platform's `scripts/domain-ci.sh`, without unit tests.
