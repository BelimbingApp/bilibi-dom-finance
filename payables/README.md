# Payables

`finance/payables` owns a company's supplier documents. This NP1 slice mirrors Bilimbi.Finance.Payables settings, tax codes, bills, debit notes and document approvals. It has no pages or agent operations; the bills pages land in NP4, matching in NP2 and payments/reports in NP3.

## Facade

Resolve `Payables` through `finance/payables.payables`. Every call takes the sealed scope and an explicit company ID; every call authorizes through Core Company. Writes hold the company lock on an audited `MutationTransaction`, refusing an archived company. Actor and impersonator always come from the scope.

| Calls | Capability |
| --- | --- |
| `get_settings`, `list_tax_codes`, `list_settings_changes`, `list_tax_code_changes`, `get_document`, `list_documents`, `page_documents` | `finance.payables.view` |
| `put_settings`, `create_tax_code`, `update_tax_code` | `finance.payables.settings.manage` |
| `create_bill`, `create_debit_note` | `finance.payables.documents.create` |
| `approve_document`, `void_document` | `finance.payables.documents.approve` |
| `post_document` | `finance.payables.documents.post` |

Configuration and recording additionally read the ledger (`finance.ledger.view`); approvals ask `Ledger::check_approval` on their own transaction with the creator and any impersonator as preparers. Posting additionally requires `finance.ledger.entries.post`. Ledger refusals, including same-person approvals and closed periods, propagate as `PayablesError::Ledger`. The eight reference capability keys are contributed to `tenant_owner`; payment and matching verbs are reserved for the following slices.

Settings name an active liability account and optional quantity/price tolerances (percent or amount, nonnegative with at most four decimal places). Tax codes have an immutable code, name, percentage below 1000 with up to four decimal places, active flag and an active ledger account. Each actual change captures its previous/new values, optional reason, actor, impersonator and database time in an append-only history. An unchanged write records nothing. Histories page oldest first by `after` ID, with `limit` from 1 to 500 and `next_after`.

## Documents

A bill records a party snapshot, the party's own number, dates, description and positive lines against active asset/expense accounts. A duplicate party number is refused until the original is voided. Due date defaults to document date and cannot precede it. A debit note copies a posted bill's party, cannot precede the bill, and cannot exceed its remaining balance either when recorded or when posted. Before payments land, that balance is the bill less all posted debit notes. NP3 extends the minimum future balance with payment allocations and reversal dates.

Amounts are `Money`, revalidated against the book's minor units. Tax snapshots group lines by code in first-use order, copying the code, rate and account. Computed tax rounds half up once per code; explicit party amounts may override it, with unused or repeated codes refused. Later tax-code changes never change a recorded document.

Documents move `draft → approved → posted`, or `draft/approved → voided` with a reason. Approval invokes the ledger's separation of duties, including impersonation. Approval makes attachments final; a void also keeps them. All content and lines remain immutable. The database stamps each transition and assigns a gapless document number per company/type only on posting.

Posting calls `Ledger::post_entry_in` on the document's own transaction, owned as `finance/payables`, `document:<id>`. A bill debits line/tax accounts and credits the current payable account. A debit note mirrors this against its bill's original payable account even after configuration changes; only that settlement account may be inactive. The ledger validates the document's actual date, and a refusal rolls everything back without taking a document number. No ledger table is read or written here.

`list_documents` is ordered by `(document_date,id)`, with a following cursor, date/type/status/party filters and limit 1–500 (default 100). `page_documents` is newest first, with case-insensitive party-name or posted-number search, page and sizes 25/50/100. Reads return complete documents including lines, tax snapshots, correction IDs and a posted bill's outstanding balance.

## Source documents and later slices

The contributed `finance/payables/document` source reads on the connection Attachments hands it and reauthorizes `finance.payables.view`. It labels a posted document by number and a draft by record ID. Attachments remain replaceable/removable only while the document is a draft. Dates cannot change after recording, so no retention refresh is needed at approval.

`external_order_line_id` is stored uninterpreted. An actual `order_line_id` currently refuses with `names no installed order source`. `TODO(domain events)` in the recording seam marks NP2's local order/receipt projection; this module has no cross-domain dependency, port or SQL.

## Proof

Run the mounted disposable database walkthrough: `DATABASE_URL=… cargo run -p bilibi-finance-walkthrough-fixture`. The payables scenarios prove configuration histories, immutable tax snapshots, bills and notes, independent approvals, atomic owned posting and rollback, attachment finality, authorization, archived companies, period/account refusals, concurrent posting and PostgreSQL's own guards. The composition gate is Platform `scripts/domain-ci.sh`.
