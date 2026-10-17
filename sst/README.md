# SST returns: setup and figures

`finance/sst` mirrors Bilimbi's `Bilimbi.Finance.Sst` at `4abfaf829321f8540581d833752a086b21b45ba9`: Malaysia's Sales and Service Tax return, form SST-02, for a company registered for sales tax, service tax, or both. NS1 owns the whole schema, the registration and its numbered versions, which SST-02 box each receivables and payables tax code reports in, the form itself, the taxable periods of the registration's cycle, and a return's figures computed from posted documents and reconciled against the tax accounts' ledger postings. Opening, adjusting, discarding, finalising, amending, reproducing and exporting returns, and the evidence datasets, belong to NS2; the Returns, Return and Setup pages, the CSV download and the print page, and the two menu items, belong to NS3. This slice contributes no routes, menu items, agent operations, CSS or JavaScript.

Filing the return electronically to MySST, paying the tax, and the penalty of item 15 are out of scope, as in Bilimbi: a return is prepared, reviewed, finalised and exported here, and filed by the company.

## Sources

The form's layout and rules come from the Royal Malaysian Customs Department (RMCD) and the MySST portal, as Bilimbi read them on 13 October 2026:

- **Form**: Borang SST-02 as amended on 2 October 2026, <https://mysst.customs.gov.my/wp-content/uploads/2026/10/BORANG-SST-02-AMENDMENT-02102026.pdf>.
- **Completion guide**: *Panduan Mengisi Penyata SST-02 (Secara Manual/Pindaan)*, 31 May 2026, <https://mysst.customs.gov.my/wp-content/uploads/2026/07/Panduan-Mengisi-Penyata-SST-02-Manual-Pindaan-BM-2026-2.pdf>.
- **Guide on Return and Payment**, version 3 (13 October 2020), RMCD's specific guide for sales tax and service tax.
- **Sales Tax Act 2018** (Act 806) and **Service Tax Act 2018** (Act 807), with their 2018 regulations.
- **Rates**: the Sales Tax (Rate of Sales Tax) Order 2025 and the Service Tax (Rate of Tax) Order 2018 as amended, linked from <https://mysst.customs.gov.my/sst-orders/>.

Rates are a company's data, never code: each tax code carries its rate, and the boxes of items 11(a) to 11(d) check that the codes mapped to them carry 5%, 10%, 6% and 8%, the rates the form prints. A new rate the form gains is a new box in `src/form.rs`, reviewed like code.

## Facade

Resolve `Sst` at `finance/sst.sst`. Every data call takes a sealed `Scope` and an explicit company ID and checks the capability and the company's reach when it runs. Writes open `Ledger::write`: the capability before and inside the audited `MutationBoundary`, the company's writable row lock (an archived company is `CompanyArchived`, and one company's SST writes run one at a time), then every inserted row recorded as a `Change`. The actor and any impersonator come only from the scope. Documents and ledger figures are read only through the `Receivables`, `Payables` and `Ledger` facades with the same scope, so a call that lists tax codes or computes figures also needs `finance.receivables.view`, `finance.payables.view` and `finance.ledger.view`; those reads run before any lock is held.

| Call | Capability | Result |
| --- | --- | --- |
| `Sst::boxes(tax_type)`, `Sst::mappable_boxes(source)` | none | The form's boxes in order; the boxes a receivables or payables tax code may be mapped to, across both forms. |
| `get_registration(scope, company)` | `finance.sst.view` | The current registration (the latest version), or `None`. |
| `list_registrations(scope, company)` | `finance.sst.view` | Every version, oldest first. |
| `register(scope, company, RegistrationInput)` | `finance.sst.settings.manage` | The next version, or the current one unchanged when nothing changes. |
| `taxable_periods(scope, company, date, count)` | `finance.sst.view` | Up to `count` (1 to 24, default 6) periods of the current cycle, newest first, ending with the one containing `date` (default today, UTC), each with its due date. Empty before any registration. |
| `list_tax_codes(scope, company)` | `finance.sst.view` | Every receivables and then payables tax code with its current mapping or `None`. Also needs both modules' view. |
| `map_tax_code(scope, company, MappingInput)` | `finance.sst.settings.manage` | The code's next mapping, or its current one when the box is unchanged. Also needs the source module's view. |
| `list_mappings(scope, company, limit, after)` | `finance.sst.view` | Every mapping recorded, oldest first; `limit` 1 to 500 (default 100), `after` the last id read. |
| `preview_return(scope, company, tax_type, period_start, Pins)` | `finance.sst.view` | The `Figures` a return of that tax for the period starting `period_start` would show now, without opening one, optionally pinned. Also needs the three view capabilities. |

All five reference capability keys are declared verbatim with the module's own verbs `sst-prepare`, `sst-finalise` and `sst-amend` (`view` and `manage` are the platform's), and granted to `tenant_owner`. Preparing, finalising and amending are separate capabilities so one person can prepare a return and another finalise it; they have no callable workflow until NS2.

Refusals are `SstError` variants in the words Bilimbi's pages show: `Unauthorized`, `CompanyNotFound`, `CompanyArchived`, `BookNotOpen`, `NotRegistered`, `NotRegisteredForTax`, `NotATaxablePeriod`, `InvalidTaxType` (from `TaxType::from_str`), `TaxCodeNotFound`, `InvalidSource`, `InvalidOption`, or `Invalid(Vec<FieldError>)` with Bilimbi's changeset messages. A sibling facade's own refusal stays its own variant (`Ledger`, `Receivables`, `Payables`), except reach and grant refusals, which read as this module's.

## Registration

A registration holds the sales tax and service tax registration numbers (at least one, each trimmed, upcased and shaped `A00-0000-00000000`: a station letter and two digits, the year and month of registration, and an eight-digit serial), the taxable period cycle (`period_start` and `period_months`, 1 to 12, 2 by default) and `service_tax_basis`. It is never changed: `register` records the next version, numbered by the database, with the actor, an optional `reason` (at most 500 characters) and the database's time, carrying over every field `RegistrationInput` leaves `None`; a blank number removes it. A call that changes none of the numbers, the cycle or the basis records nothing, even with a new reason. Each return (NS2) keeps the version it was prepared under.

**Taxable periods** (`src/periods.rs`). A taxable period is two months, the first running from the date a person is registered to the end of the month after it (section 25(1) of both Acts); the Director General may approve another length (section 25(2) and (3)). The first period runs from `period_start` to the end of the month `period_months` months on, counting its own month; each later one is `period_months` whole calendar months. A return is due on the last day of the month after its period ends (section 26(1)), or 30 days after it on a varied cycle (section 26(2)); a due date on a weekend or holiday moves to the next working day in practice, which is not computed here.

**Service tax basis.** Service tax is due when payment is received (section 11(1) of the Service Tax Act), or, unpaid twelve months after the invoice date, on the day after (section 11(2)); a registrant may apply to account for it when invoiced instead (section 11(1A)). `service_tax_basis` is `payment` (the default) or `invoice`. Sales tax is always due when goods are sold, by the document date.

The cycle and the basis change only from a `period_start` after the last finalised return's period, so a reported period stays the period it was and nothing is counted under two bases (`Invalid` on `period_start`, and the database refuses the same); the numbers may change at any time. A period is read under the latest version whose cycle starts on or before it.

## Tax code mappings

Each receivables and payables tax code is mapped to the SST-02 item it reports in, or to none (deliberately not reported, such as a code for a tax other than SST). A mapping is never changed: `map_tax_code` records the next one, numbered by the database, with the actor, an optional reason and the time, and the latest for a code applies. Figures read the mappings through a mapping number, so a reading pinned at an earlier number routes rows as they were routed then. A code with tax in a period and no mapping is listed in `unmapped` and will stop a return finalising (NS2); it is never silently left out.

| Source | Boxes a code may map to |
| --- | --- |
| Receivables | 11(a), 11(b), 17, 18(a), 18(b)(1) to 18(b)(3)(iii), 18(d) for sales tax; 11(c), 11(d), 18(c)(1) to 18(c)(4) for service tax |
| Payables | 19, 20, 21 (Schedule C purchases) for sales tax; 11(c), 11(d) (taxable services acquired or imported) for service tax |

## The form

`src/form.rs` holds each tax's boxes in the form's order; SST-02 is filed separately for sales tax and service tax, so each return is of one `TaxType`. A box is `Rated` (a value and its tax at the printed rate), `Specific` (item 17, any rate), `Value` (no tax), `Tax` (a tax amount only) or `Total` (items 12 and 14).

| Item | Sales tax | Service tax |
| --- | --- | --- |
| 11 | (a) goods at 5%, (b) goods at 10%: value and tax | (c) services at 6%, (d) services at 8%: value and tax; (e) Group H item 1 cards: tax |
| 12 | 11(a) + 11(b) + 17 | 11(c) + 11(d) + 11(e) |
| 13 | (a) credit notes, (b) sales tax deduction, (d) bad debt relief; 13A excess deduction claimed earlier | (a) credit notes, (c) service tax deduction, (d) bad debt relief |
| 14 | 12 − 13(a) − 13(b) − 13(d) − 13A | 12 − 13(a) − 13(c) − 13(d) |
| 17 | Second Schedule goods at specific rates: value and tax | — |
| 18 | (a) exports and Special or Designated Areas; (b)(1) to (b)(3)(iii) Schedule A, B and C exemptions; (d) section 35(3) or 61A: values | (c)(1) to (c)(4) exempted and non-taxable services: values |
| 19 to 21 | Schedule C purchases: values | — |

Item 14 follows the completion guide's formula for each tax; the form's printed formula leaves out 13(d), which the guide says is deducted once the Director General approves the relief. Items 15 and 16 (penalty), Part B1's lines per item and Part F's declaration are completed when the return is filed. Item 13A's key is `13aa`, since keys are lowercase.

## Figures

`preview_return` reads, at one ledger entry number:

- `Ledger::account_totals` over the period, which names the `through_entry_number` (or reproduces the pinned one);
- `Receivables::tax_totals` and `Payables::tax_totals` at that entry number: the tax on documents issued or posted in the period, summed by PostgreSQL per document kind and tax code;
- for service tax on the payment basis, `Receivables::received_tax_totals` at the same number instead of receivables' invoices (the codes mapped to a service tax box): the tax share of each payment received in the period before its invoice fell due under the twelve-month rule, less reversals, the share an invoice still owed when its twelve months ran out, and credit notes issued after that on tax already due.

Each row goes to the box its code maps to: an invoice or bill adds its value and tax; a credit note on an invoice of the same period, or a payables debit note, subtracts them; a credit note on an earlier period's invoice (or one on tax already due) deducts its tax in item 13(a), where the guide puts credit notes for tax already paid, except into a value-only box, which nets it in place. A row whose code maps to the other tax's form is left to that return; one mapped to none is listed with no box and counted nowhere. Each row's contribution keeps the columns' four places, each box is the exact sum of its rows rounded half up to the book's minor units, and items 12 and 14 are computed from the rounded boxes.

`Figures` carries `boxes` (each with `taxable_amount` and `tax_amount`, `None` where the box takes no such column), `tax_payable` (item 14), `sources` (every counted row with its box, mapping number and signed contribution), `unmapped`, `rate_mismatches` (rows mapped to a rated box of another rate, or to a value box with tax), the currency and minor units, and the `through_entry_number` and `through_mapping_number` it was read at. The same `Pins` read the same figures: posted documents never change, and a later posting carries a later entry number.

**Ledger reconciliation.** For each tax account a code of the form (or an unmapped code) posted to in the period, `accounts` compares the account's ledger debits and credits over the period with what every document posted to it: receivables invoices credit their tax and credit notes debit it; payables bills debit theirs and debit notes credit it. The `difference`, ledger credits less debits less the documents' credits less debits, is always shown, never hidden: an entry no document explains, such as the payment of an earlier return, a manual journal, or another module's posting. Under the payment basis the return's service tax differs from the documents' by design; the reconciliation still compares documents with postings.

## Database

Migration `finance-sst-001-sst` is Bilimbi's `20261015090000_create_finance_sst` end state: `finance_sst_registrations`, `_mappings`, `_returns`, `_return_events`, `_return_adjustments`, `_return_boxes`, `_return_sources` and `_return_accounts`, with nine functions and ten trigger declarations (thirty triggers). Rows of one company reference each other through composite `(id, company_id)` foreign keys; tax code and ledger account ids are plain columns, validated by their owners' facades. PostgreSQL enforces what the facade checks, so no other writer can break it:

- every table is append-only: no UPDATE, DELETE or TRUNCATE;
- triggers number each company's registration versions, mappings, returns, return events and adjustments without gaps and stamp their times, replacing whatever the writer sent, and give a `finalised` event its version, the count of the period's finalised returns of that tax;
- a registration's cycle or basis changes only from after the last finalised period;
- per tax, one open return of a period, no overlapping periods, a period's first return amends nothing and every later one amends its latest finalised return with a reason;
- a return is recorded with its `opened` event (a deferred check refuses one without); `finalised` and `discarded` need an open return, a `finalised` event carries its numbers, currency and tax payable, a deferred check refuses one without its box figures, and a `discarded` one needs a reason;
- a finalised return's boxes, sources and accounts are inserted only in the transaction that finalises it; an account row's difference must follow from its debits and credits;
- an adjustment is recorded only on an open return, and a withdrawal names an adjustment of the same return, at most once.

The numbering triggers rely on the facade's company write lock to serialize a company's writes; a concurrent writer that takes the same number fails on the unique index.

## Parity notes

- Inputs are typed (`RegistrationInput`, `MappingInput`, `TaxType`, `Pins`) instead of Bilimbi's attribute maps; `None` means "carry over" or "latest", as an absent key does in Bilimbi.
- Bilimbi reads the source module's tax codes inside the write's transaction. Here they are read through the owner's pooled facade before the write takes the company lock, as `AGENTS.md` asks for reads with no `_on` variant: tax codes are never deleted, and a code deactivated in between still reports what it carried.
- The facade-level refusal of a cycle or basis change into a reported period is ported but first reachable in NS2, once a return can be finalised; NS1's proof shows the database's own refusal.

## Running proof

From the mounted Platform root, on a fresh disposable database:

```sh
DATABASE_URL=postgres://…/disposable cargo run -p bilibi-finance-walkthrough-fixture --bin sst
```

The binary migrates and installs the disposable walkthrough fixture and then, through the real facades only: refuses figures before any registration, registrations without a number, of another shape, with a thirteen-month cycle, an unknown basis or no first period, and registrations without the settings grant; records four versions (lowercase numbers upcased, a service tax number added with the cycle carried over, a no-op call that records nothing, the invoice basis from March, a varied monthly cycle from July) and lists their two-month and monthly periods with due dates; refuses periods that start mid-cycle or before the registration. It then sets up Receivables and Payables tax codes, issues invoices, a same-period and a prior-period credit note and a payment, posts a bill, a debit note and a manual tax-account entry; refuses mappings to an unknown source, an unknown code, a box of the wrong source and a total; records seven mappings (one deliberately not reported) and pages them; and reads the January–February sales tax figures (boxes 11(a), 11(b), 18(a), 19, items 12 and 14, an unmapped code, a not-reported code, the ledger difference of the manual entry), the service tax figures on the payment basis (the received share of a part-paid invoice plus acquired services net of a debit note), a rate mismatch and its exact reproduction at the earlier mapping number, a late posting that changes the live figures but not the pinned ones, the March–April 13(a) deduction for a prior-period credit note and service tax on the invoice basis. It finishes with grant withdrawal mid-run, another tenant, an archived company, an audit mutation for every row, and PostgreSQL's own refusals of each trigger rule, each rolled back. It adds no production seeds and no unit tests; the repository gate remains the Platform's `scripts/domain-ci.sh`.
