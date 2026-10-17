# Payables

`finance/payables` owns a company's supplier documents. It mirrors Bilimbi.Finance.Payables settings, tax codes, bills, debit notes, document approvals, payment requests and payments, including aging, tax totals and control-account tie-out. It has no pages or agent operations; the bills pages land in NP4 and matching in NP2.

## Facade

Resolve `Payables` through `finance/payables.payables`. Every call takes the sealed scope and an explicit company ID; every call authorizes through Core Company. Writes open through `Ledger::write`, holding the company lock on an audited `MutationTransaction` and refusing an archived company. Book and account validation use the ledger's authorized `get_book_on`/`list_accounts_on` on that same connection. Actor and impersonator always come from the scope.

| Calls | Capability |
| --- | --- |
| `get_settings`, `list_tax_codes`, `list_settings_changes`, `list_tax_code_changes`, `get_document`, `list_documents`, `page_documents` | `finance.payables.view` |
| `put_settings`, `create_tax_code`, `update_tax_code` | `finance.payables.settings.manage` |
| `create_bill`, `create_debit_note` | `finance.payables.documents.create` |
| `approve_document`, `void_document` | `finance.payables.documents.approve` |
| `post_document` | `finance.payables.documents.post` |
| `submit_payment` | `finance.payables.payments.submit` |
| `approve_payment`, `approve_payment_with_override`, `reject_payment`, `reverse_payment` | `finance.payables.payments.post` |
| `get_payment_request`, `list_payment_requests`, `get_payment`, `list_payments`, `aged_payables`, `tax_totals`, `tie_out_control_accounts`, `evidence_page` | `finance.payables.view` |

Configuration and recording additionally read the ledger (`finance.ledger.view`); approvals ask `Ledger::check_approval` on their own transaction with the creator and any impersonator as preparers. Posting additionally requires `finance.ledger.entries.post`. Ledger refusals, including same-person approvals and closed periods, propagate as `PayablesError::Ledger`. The eight reference capability keys are contributed to `tenant_owner`; matching verbs are reserved for NP2.

Settings name an active liability account and optional quantity/price tolerances (percent or amount, nonnegative with at most four decimal places). Tax codes have an immutable code, name, percentage below 1000 with up to four decimal places, active flag and an active ledger account. Each actual change captures its previous/new values, optional reason, actor, impersonator and database time in an append-only history. An unchanged write records nothing. Histories page oldest first by `after` ID, with `limit` from 1 to 500 and `next_after`.

## Documents

A bill records a party snapshot, the party's own number, dates, description and positive lines against active asset/expense accounts. A duplicate party number is refused until the original is voided. Due date defaults to document date and cannot precede it. A debit note copies a posted bill's party, cannot precede the bill, and cannot exceed its remaining balance either when recorded or when posted. That balance is the bill less posted debit notes and allocated payments, counting reversal dates. Recording and posting a debit note checks the minimum balance on every later date, so a backdated note cannot overdraw a bill that a later payment temporarily settled.

Amounts are `Money`, revalidated against the book's minor units. Tax snapshots group lines by code in first-use order, copying the code, rate and account. Computed tax rounds half up once per code; explicit party amounts may override it, with unused or repeated codes refused. Later tax-code changes never change a recorded document.

Documents move `draft → approved → posted`, or `draft/approved → voided` with a reason. Approval invokes the ledger's separation of duties, including impersonation. Approval makes attachments final; a void also keeps them. All content and lines remain immutable. The database stamps each transition and assigns a gapless document number per company/type only on posting.

Posting calls `Ledger::post_entry_in` on the document's own transaction, owned as `finance/payables`, `document:<id>`. A bill debits line/tax accounts and credits the current payable account. A debit note mirrors this against its bill's original payable account even after configuration changes; only that settlement account may be inactive. The ledger validates the document's actual date, and a refusal rolls everything back without taking a document number. No ledger table is read or written here.

`list_documents` is ordered by `(document_date,id)`, with a following cursor, date/type/status/party filters and limit 1–500 (default 100). `page_documents` is newest first, with case-insensitive party-name or posted-number search, page and sizes 25/50/100. Reads return complete documents including lines, tax snapshots, correction IDs and a posted bill's outstanding balance.

## Source documents and later slices

The contributed `finance/payables/document` source reads on the connection Attachments hands it and reauthorizes `finance.payables.view`. It labels a posted document by number and a draft by record ID. Attachments remain replaceable/removable only while the document is a draft. Dates cannot change after recording, so no retention refresh is needed at approval.

`external_order_line_id` is stored uninterpreted. An actual `order_line_id` currently refuses with `names no installed order source`. `TODO(domain events)` in the recording seam marks NP2's local order/receipt projection; this module has no cross-domain dependency, port or SQL.

## Proof

Run the mounted disposable database walkthrough: `DATABASE_URL=… cargo run -p bilibi-finance-walkthrough-fixture`. The payables scenarios prove configuration histories, immutable tax snapshots, bills and notes, independent approvals, atomic owned posting and rollback, attachment finality, authorization, archived companies, period/account refusals, concurrent posting and PostgreSQL's own guards. The composition gate is Platform `scripts/domain-ci.sh`.

## Payments and reports

`submit_payment` freezes a payment request and its positive, unique bill allocations without reserving the balance. Each bill must be posted, belong to the party and be dated on or before the payment. The paid-from account must be an active asset or liability account. Amounts are revalidated with the book's minor units. The request is numbered by the database; its single append-only decision is `posted` or `rejected` (with a required reason). `list_payment_requests` filters by status and pages by request number, default 100 and maximum 500.

`approve_payment` invokes ledger duty separation against the submitter and impersonator, then repeats every validation against current data. `approve_payment_with_override` additionally records an explicit override reason through the ledger. It debits each allocated bill's original payable account and credits the paid-from account, atomically storing the owned entry (`payment:<id>`), payment, allocations and decision. Historical payable accounts may be inactive for this settlement. A failed approval leaves the request submitted and consumes no payment number. Requests do not reserve funds; concurrent approvals serialize on the company lock.

`reverse_payment` calls the ledger's owned reversal path in the same audited write as the reversal record. Its date cannot precede the original; a payment reverses once, and a reversal cannot itself reverse. Reversals carry no allocations. A bill becomes owed again from the reversal date. Payment reads include their reversal IDs; lists filter by party, document (including its payment reversals), inclusive dates and a `(payment_date,id)` cursor, bounded at 500.

`aged_payables` groups positive outstanding bills by party and due date, with default 30/60/90-day boundaries and a not-yet-due bucket. Negative balances are explicit exceptions. Its posting fence defaults to the last Payables entry and reproduces past readings despite later backdated postings. `tax_totals` groups immutable posted tax snapshots by type, code, rate and account, with optional date window and posting fence. Debit notes retain their positive totals under their own type.

`tie_out_control_accounts` first reads `Ledger::account_totals_by_owner`, then reads the subledger at that exact posting number. It includes every historical/configured payable account, shows credits less debits, and lists other owners' postings as differences. No ledger table is read. Pass its `through_entry_number` to other report columns.

The `finance/payables/payment` attachment source describes a payment request from submission onward, so pre-approval remittance evidence stays linked to its posted payment. Its attachments can be replaced or removed while submitted and become final when posted or rejected. `evidence_datasets` and authorized `evidence_page` export all twelve owned tables through frozen field lists, ID-ordered pages of 1–500, with exact numeric text and UTC timestamps. Matching tables will be added by their owner slice. The NP3 walkthrough exercises the workflow, refusals, temporal balances, pinned reports, tie-out, evidence, attachments and database immutability.
