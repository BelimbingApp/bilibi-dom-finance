# E-invoice

Module ID: `finance/einvoice` (Bilimbi `Bilimbi.Finance.Einvoice`). The fiscal
documents a company issues through a clearance authority. This version holds
the schema, the company's setup (settings, credentials, signing certificate
with its facts, and supplier profile), the `Authority` contract with the one
authority this release clears documents with (MyInvois) and its rules: the
draft a caller asks to issue and its validation, the UBL 2.1 JSON builder with
its byte-exact writer, the JSON XAdES signer, and the authority's code lists
with each company's overrides; and clearance: issuing a document, submitting
it through the MyInvois client on Base Integration with the submit and poll
jobs on Base Queue, refreshing its status and cancelling it; and the ledger
bridge: a validated document's one owned entry and a cancelled one's
reversal, retried from the reconciliation read, source links to the entry
Receivables or Payables already posted, the monthly consolidated document,
the printable PDF with its validation link's QR code, and the auditor's
evidence datasets. The pages follow (slices NE5 and NE6). Other modules and
Domains never read its tables.

## Public API

Resolve the `Einvoice` service (`finance/einvoice.einvoice`) from the registry
and call it with a sealed `Scope` and an explicit company id.

| Function | Capability | Does |
| --- | --- | --- |
| `Einvoice::code_lists` | none | The lists the authority publishes, sorted |
| `list_codes` | `finance.einvoice.view` | The company's active codes in one list, code to description: the shipped list with the company's overrides applied; `UnknownList` for a list the authority lacks |
| `override_code` | `finance.einvoice.settings.manage` | Renames a code, names one the shipped list lacks, or retires it, for that company only; audit action `finance.einvoice.code.overridden` |
| `setup` | `finance.einvoice.settings.manage` | What the company has configured, never a secret (see Settings), with the signing certificate's facts |
| `Einvoice::authority` | none | The `Authority` every company's documents are cleared with |
| `Draft::validate` | none | A draft's shape and the authority's rules, with the derived amounts (see Drafts) |
| `issue_document` | `.documents.issue` | Records a valid draft as an `issued` document; `Invalid`, `CodeNumberTaken`; audit action `finance.einvoice.document.issued` |
| `get_document` | `.view` | A `DocumentView`: the document, its lines, events, status, identifiers, validation link, cancellation deadline, attention flags and whether a submit job waits |
| `submit_document` | `.documents.submit` | Queues the submit job as the caller (answers its ID); `NotSubmittable`, `SourceTaken`, `MissingCredentials`, `SigningKeyRequired`, `AlreadyQueued`; audit action `finance.einvoice.submission.queued` |
| `refresh_status` | `.documents.submit` | Reads the status at the authority and records what changed; `NotSubmitted`, `Authority(code)` |
| `cancel_document` | `.documents.cancel` | Cancels a valid or rejected document at the authority with a reason; `NotCancellable`, `CancellationPeriodOver`, `ActiveReferencingDocuments`, `InvalidReason`, `CancellationRefused`, `RateLimited`, `AuthorityUnavailable` |
| `test_connection` | `.settings.manage` | Logs in afresh with the company's current credentials; audit action `finance.einvoice.connection.tested` |
| `run_submission`, `run_poll` | `.documents.submit` | The submit and poll jobs' bodies (see Clearance), callable directly for one attempt |
| `retry_posting`, `retry_reversal` | `.documents.submit` and `finance.ledger.entries.post` | Posts a valid document's entry, or reverses a cancelled one's, after a failure (see Ledger bridge); `NotPostable`, `NotReversible`, `LedgerFollowUpFailed(reason)`; audit action `finance.einvoice.posting.retried` |
| `reconciliation` | `.view` | `unposted` (valid without an entry) and `unreversed` (cancelled with an entry standing) documents, newest first, each with its entries and the last failure's reason |
| `consolidation_candidates` | `.view` | What a month's consolidated document would cover at `now`: the window, the eligible documents, those the authority's limits refuse with the reason, and the totals |
| `issue_consolidated_document` | `.documents.issue` | Records the month's consolidated document at `now` (see Consolidation); audit action `finance.einvoice.document.consolidated` |
| `generate_pdf` | `.view` | Renders the document's PDF on Base Artifacts once per state and records `pdf_generated`; audit action `finance.einvoice.pdf.generated` |
| `read_pdf` | `.view` | A stored PDF's bytes and download name, for the download route |
| `qr_code_svg` | none | The validation link's QR code as an inline SVG |
| `Einvoice::evidence_datasets`, `evidence_page` | `.view` | The auditor pack's frozen exports of this module's tables (see Evidence) |

Every call authorizes its capability for the scope's actor and the target
company at the moment it runs (Core Company's `authorize_target`): a missing
grant, or a sibling company without `admin.company.tenant-wide.manage`, is
`EinvoiceError::Unauthorized`; a company the scope cannot see is
`CompanyNotFound`. A write then holds the company's row lock inside an audited
`MutationBoundary` transaction, so an archived company is `CompanyArchived`.
Refused values are `Invalid(Vec<FieldError>)` with Bilimbi's changeset words
(`must be 1 to 16 letters, digits, dots, or dashes`, `should be at most 500
character(s)`), each at its field's path (`code`, `buyer.address.state_code`,
`lines[2].tax_rate`).

## Drafts

`Draft` is a document a caller asks to issue, as plain attributes, before it
is recorded: the document's type and code number, its issue instant (now when
none is given, never in the future), currency and exchange rate, the
counterparty (`buyer`, or `supplier` on a self-billed document, the company's
own profile taking the other side and never given), optional payment and
billing-period details, the original document a note corrects, and one to a
thousand lines. Amounts are derived, never taken: a line's subtotal is
quantity times unit price, its amount is the subtotal less its discount plus
its charge, and its tax is that amount times the rate, each rounded half up
to two places; the document's totals are the sums. An amount finer than its
column is refused, not rounded. The one exception is a consolidated
document's line, which carries its tax, the sum of its sources' tax.

`Draft::validate(now, &authority, &context)` checks the shape every authority
shares (Bilimbi's `Draft` changeset), then the authority's own rules
(`Authority::validate`): codes against the company's lists (`list_codes`, one
list per entry of `context.codes`), the fields its rules require, a company
profile (`context.supplier`, the setup's) missing what every document carries,
a note whose original (`context.original`) is not one the authority lets it
reference, and the consolidated document's own shape (`context.consolidated`).
It answers the `ValidDraft` with every line's derived amounts, or every
`FieldError` in Bilimbi's words. The walkthrough prints both for invoices,
notes, self-billed and consolidated drafts.

## Clearance

A recorded document is cleared with the authority by the submit and poll jobs
(`finance/einvoice-submit`, 100 attempts, and `finance/einvoice-poll`, 20
attempts), Base Queue workers queued with `enqueue_for`, so each runs as the
person who asked: the queue re-proves that person and their
`.documents.submit` grant on every attempt, every write of a job opens with
`Execution::begin` (which fences the job's lease), and the arguments are the
company and document IDs only. Each outcome is a new status event with the
authority's answer beside it, and each outcome the authority decides is also
an audit action; nothing edits a document. No transaction is held across a
call to the authority.

- **Submitting.** A submission row holding the exact bytes (signed when the
  company's format version is) commits before the HTTP call, so an attempt
  that dies mid-flight still leaves what it sent. The answer is then recorded
  under the company lock, only while the document is still submittable:
  `submitted` (with the submission UID and document UUID; the poll job is
  queued; action `finance.einvoice.submitted`), `refused` (action
  `finance.einvoice.refused`), `duplicate` (action
  `finance.einvoice.submission.duplicate`; the job stops with
  `duplicate_submission` for an operator), `rate_limited` (retry
  `rate_limited`) or `unavailable` (retry `authority_unavailable`). A document
  that cannot be sent records `unsendable` and stops the job with its reason
  (`missing_credentials`, `signing_key_required`, `source_taken`, ...).
- **Never resubmitting blindly.** After a duplicate, or an outage that may
  have hidden an accepted request, and before any retry that finds an earlier
  attempt without a final answer, the authority is asked for the document by
  its code number among what was sent from a minute before the oldest such
  attempt; what it holds is recorded as `submitted`. Only a clean not-found
  lets the bytes go out again; a search that cannot answer records
  `search_deferred` and retries without sending.
- **Overdue.** The queue retries after `min(3600, 2^attempt)` seconds, so a
  hundred attempts span about four days. Once the authority has been
  unavailable for its outage allowance (72 hours) since the document's first
  failed attempt, an `overdue` event and the action
  `finance.einvoice.submission.overdue` give the operator notice once; the job
  keeps retrying.
- **Polling.** The poll job reads the submission's progress up to ten times,
  four seconds apart, and retries `in_progress` while the authority is still
  processing. Then it reads the document's status and records `valid` (with
  the long ID that completes the validation link; action
  `finance.einvoice.validated`) or `invalid` (with the failed validation
  steps; action `finance.einvoice.invalid`). A 404 right after a submission is
  retried as `not_yet_available`; another refusal records `status_unreadable`.
- **Refreshing** records what changed: a buyer's rejection request becomes
  `rejected` (action `finance.einvoice.rejected`), a cancellation on the
  portal `cancelled`, and an unchanged status `status_checked`.
- **Cancelling** is guarded before the authority is asked: a valid or
  rejected document, inside the 72 hours after validation, with no note the
  authority holds (submitted, valid or rejected) referencing it, and a reason
  of 1 to 300 characters. The authority's refusal is recorded as
  `cancellation_refused`. Each outcome is recorded only while the document,
  read again under the company lock, is still cancellable.

Once a `valid` (or `rejected`) status commits, the bridge posts the
document's entry; once `cancelled` commits, it reverses it (see Ledger
bridge). A document's view flags `Overdue`, `RejectionRequested`, `Unposted`,
`PostingFailed`, `Unreversed` and `ReversalFailed` for an operator.

## Ledger bridge

`posting.rs` (Bilimbi `Posting`). A document the authority validated posts
one balanced entry through the ledger's `post_entry_in`, as the person the
work runs for (the poll job's, the refresh's or the cancellation's), dated
its issue date in UTC (a consolidated document's, the last day of the month
it reports), described `E-invoice <code> (<type>)`, with its code number and
authority UUID as the `reference`. The accounts are the company's
`finance.einvoice.posting.*` codes in its own chart; the authority's
`posting_rule` says which three a type uses. The counter-account
(receivable, payable or settlement) takes the rule's side for the payable
amount; the net (revenue or expense) and tax accounts take the other side,
and a tax of zero posts no tax line. A sale debits receivable; a credit note
mirrors it; a refund note credits settlement; a self-billed purchase credits
payable and debits expense and purchase tax.

The entry's owner is `finance/einvoice` `document:<id>`, so the ledger
refuses a second entry for the document and a reversal by anyone else
(`OwnedBy`). Its ID is recorded on a `posted` event, and the reversal's on a
`posting_reversed` event, each in the transaction that posts it, under the
company lock; the database refuses a second of either. A refusal (book not
open, the book's currency differs, `posting accounts not set: ...`,
`accounts not in the chart: ...`, a closed period, a missing ledger grant) is
recorded afterwards, in its own transaction, as `posting_failed` or
`reversal_failed` with the reason, and the document waits on
`reconciliation` until `retry_posting` or `retry_reversal` succeeds. Only a
database failure that may pass at once (a lost connection, lock contention, a
deadlock, a serialization failure) is tried a second time, and recorded as
one failure that says so. A document posts and reverses once: asking again
is unchanged. A clearance never fails because its follow-up did.

A document issued with a `source` never posts or reverses (see Source
links): its accounting and its correction are the source module's.

## Source links

`Draft.source` names the transaction the document is the tax document of,
when another Finance module already recognised it in the ledger: the
module's descriptor ID (`finance/receivables`, `finance/payables`), its
record (`document:17`) and the entry that record posted. `issue_document`
holds it to the ledger through `get_entry_on` (so the caller needs
`finance.ledger.view`): the entry exists, that module and record own it, it
is neither a reversal nor reversed, it is in the document's currency, and its
debits equal the document's total; and no other document that stands
(submitted, valid, rejected or consolidated, or with an attempt the authority
may hold) links the same source, which the database refuses too. Each
problem is a `source` field error in Bilimbi's words (`ledger entry 12 is
not owned by finance/receivables document:3`). A note correcting a linked
original needs its own `source`: the record that module corrected it with.
The view lists `source_links` and says `recognised_by_source`.

## Consolidation

`consolidation.rs` (Bilimbi `Consolidation`). The sources of a month's
consolidated document are the company's documents issued in the month (the
authority's calendar: MyInvois' UTC+8 month) in the current environment,
never submitted on their own, not held by another consolidated document, not
waiting in a submit job, and within the authority's limits (an invoice, in
ringgit, at most RM10,000). It is issued from the month's end for seven days
(`PeriodNotEnded`, `ConsolidationWindowClosed`) at the `now` the caller
passes (the database still stamps `recorded_at`), with a line per source and
tax whose amount and tax are that source's exact sums, to the general public
buyer with classification `004` and the month as its billing period. It
records which source each line carries and a `consolidated` event on each
source, which then cannot be submitted alone; a refused, invalid or
cancelled consolidated document releases them (`released`) and is never sent
again. Linked sources pass their links on, and linked and unlinked sources do
not mix (`MixedSourceLinks`). The view of a consolidated document lists its
`sources` with their line numbers; a source names its `consolidated_by`.

## Printable document

`pdf.rs` (Bilimbi `ArtifactOwner` and `QrCode`). `generate_pdf` renders the
document with Base Artifacts' deterministic serializer (parties, lines,
totals, status, identifiers and the validation link; characters outside
Latin-1 print as `?`) and stores it under owner `finance/einvoice`, kind
`document-pdf`, subject `document-<id>`. The adapter re-asks
`finance.einvoice.view` on every create, read and delete and never retains.
A `pdf_generated` event keeps the artifact and the last event it was
rendered from, so asking again before the document changes answers the same
artifact. Base Artifacts needs its storage root and retention period first.
`qr_code_svg` draws the validation link's QR code (black on white, no
script) for the document page.

## Evidence

`evidence.rs`: one dataset per table (`code_override`,
`consolidated_source`, `document`, `line`, `source_link`, `status_event`,
`submission`) with a frozen field list, ID-ordered pages of 1 to 500. The
submission's payload is exported as the text it is; credentials live in Base
Settings and are never exported.

### The MyInvois client

`myinvois/client.rs` calls the API only through Base Integration's `send`, as
the person the call acts for, so each exchange is recorded under them without
bodies or headers (system `myinvois`, owner `finance/einvoice` and the
company). Logging in (`POST /connect/token`, client credentials, scope
`InvoicingAPI`) yields a token that lives an hour; it is cached in process
memory per company, environment and client ID until two minutes before it
lapses, and a 401 drops it, so the authority's 12 logins a minute are never
spent per call. A token never reaches the database, a log or an exchange
record. The request envelopes are written by the module's ordered JSON
writer, like the documents. `myinvois/api.rs` turns each answer into the
`Authority` callbacks' meta-term outcomes (`SubmitOutcome`, `Progress`,
`Search`, `StatusRead`, `CancelOutcome`, `LoginOutcome`), keeping the
authority's answer intact. Nothing in the client retries.

## Capabilities

Five keys, verbatim from Bilimbi, under the `finance` domain, all granted to
`tenant_owner`: `finance.einvoice.view`, `.documents.issue`,
`.documents.submit`, `.documents.cancel` and `.settings.manage`. The module
declares the verbs `issue`, `submit` and `cancel` (Bilibi merges verbs across
modules). It contributes the two queue workers above and has no pages, menu items or agent operations yet. It depends on `finance/ledger` (posting, reversal, source entries) and `base/artifacts` (the PDF), where Bilimbi has the same edges.

## Settings

Company-scoped Base Settings, edited on the Settings screen under the group
`E-invoice` and guarded by `finance.einvoice.settings.manage`. No value has a
default that names a country, a tax number or a credential.

- `finance.einvoice.environment` (`sandbox` by default, or `production`),
  `.document_version` (`1.1`, signed, or `1.0`, unsigned, for sandbox testing
  before the certificate arrives) and `.expiry_alert_days` (30, 1 to 365).
- Per environment: `finance.einvoice.<environment>.client_id`,
  `.client_secret` (encrypted) and `.secret_expires_on` (`YYYY-MM-DD`).
- `finance.einvoice.certificate_pem` and `.private_key_pem` (both encrypted),
  validated as one readable X.509 certificate with an RSA public key and one
  unencrypted RSA private key (`PRIVATE KEY` or `RSA PRIVATE KEY`; PKCS#12,
  encrypted and EC keys are refused).
- `finance.einvoice.supplier.*`: name (blank uses the company's), TIN,
  registration type and number, sales and tourism tax registrations, industry
  code, business activity, phone, email and the address with the authority's
  state code and the ISO country code. The authority fixes the shape of the
  TIN, phone, industry code, registration type, state code and country code.
- `finance.einvoice.posting.*`: the account codes of the company's own chart a
  validated document posts to (`receivable_account`, `revenue_account`,
  `tax_account`, `settlement_account`, and for self-billed purchases
  `payable_account`, `expense_account`, `purchase_tax_account`).
- Platform settings (operators, `base.settings.global.manage`):
  `finance.einvoice.<environment>.api_url` and `.portal_url`, an https URL
  without a path, defaulting to the authority's published hosts. The sandbox
  API URL may also be an http URL on this machine's loopback (`127.0.0.1`,
  `[::1]` or `localhost`, with a port), where a stand-in answers sandbox calls;
  production is https only.

Encrypted values use Base Settings' envelope (`BILIBI_SETTINGS_KEY`), are masked
on screen and revealed only through its password-checked reveal. `setup`
reports whether a client secret, certificate and private key are set, and the
certificate's facts (`CertificateSetup::Facts`: issuer and subject names as
the authority writes them, serial number, the tax ID and registration number
its subject carries, validity, days until expiry, and whether the stored
private key belongs to it), what expires within the alert window
(`expiring`), and whether the supplier profile's TIN is the certificate's. It
reads the PEMs inside the facade and returns no byte of them; `Unreadable`
means a stored certificate does not parse or the settings key is unavailable.

## Authority

`Authority` is everything one clearance authority defines that needs no
network: its document types, posting rules, format shapes, versions, the
consolidation window, the validation link, its rules over a draft
(`validate`) and the payload a recorded `Document` becomes (`payload`: the
format, version and exact bytes to submit, signed at the given time when the
version is signed, `SigningKeyRequired` without a key, `UnsupportedVersion`
for a version it lacks). MyInvois is the only implementation (Guideline v4.8
and the SDK as read on 12 October 2026). Its API callbacks (`login`, `submit`,
`submission_progress`, `find_submission`, `document_status`, `cancel`) take a
`Connection` (the outgoing edge, the person, the company, its credentials and
the environment's base URL) and answer in meta-terms. The consolidated
document's shape and refusal join the trait with consolidation.

Everything MyInvois defines sits in `src/myinvois.rs` and its submodules:

- `myinvois.rs` validates a draft (invoices and notes in ringgit; a note
  references the validated original it corrects, an invoice references
  nothing; a self-billed supplier carries its industry and activity and the
  foreign-supplier TIN only outside Malaysia; a consolidated invoice goes to
  the general public buyer with classification `004`; types `01` to `05` need
  a rate above zero, `06` and `E` zero, `E` an exemption reason) and maps a
  recorded document to the builder's input, with per-tax-type subtotals and a
  note's original as a `BillingReference`. It alone defines the authority's
  field shapes, which the settings validators and the buyer's checks both
  ask, so supplier and buyer follow one rule each.
- `myinvois::ubl` writes UBL 2.1 JSON in the element order of the SDK's
  samples from a plain `ubl::Document`. It maps and never computes, which is
  why `examples/sdk_samples.rs` rebuilds all four SDK version 1.1 samples
  (`examples/sdk/`) byte for byte, figures that do not add up included:

  ```sh
  cargo run -p bilibi-finance-einvoice --example sdk_samples
  ```

- `myinvois::json` is the order-preserving, minified writer and reader
  (`Json`): an object's key order and a number's written form survive a read
  and a write (`1500.0` stays `1500.0`). The authority hashes and signs exact
  bytes, so UBL JSON is never written through `serde_json`, whose maps sort
  keys.
- `myinvois::signer` follows the SDK's "Signing a JSON document" steps: the
  document without `UBLExtensions` and `Signature`, minified, is hashed and
  signed with RSA PKCS#1 v1.5 and SHA-256; the certificate's DER is hashed;
  the signed properties carry the UTC signing time and the issuer name and
  serial number; their minified `{"Target", "SignedProperties"}` object is
  hashed. `verify` checks a signed document against the certificate it
  embeds (both digests, the certificate digest, issuer and serial, and the
  RSA signature). The example verifies the SDK's four signed samples, rebuilds
  their signed properties from the SDK certificate, and signs and verifies
  with a throwaway certificate.
- `Certificate::parse` and `PrivateKey::parse` read the PEMs a company stores:
  the DER bytes, the issuer name as .NET renders it (attributes in reverse DER
  order, comma-joined, quoted when needed), which is what both SDK
  certificates show, the decimal serial number, the validity window, and the
  subject attributes (`organizationIdentifier`, `serialNumber`) that carry the
  company's tax and registration numbers.

Foreign-currency documents are refused for now: the SDK's sample writes tax
amounts in ringgit beside document-currency totals without saying how they
convert, so the rule is confirmed against the sandbox first. The builder
already writes `TaxExchangeRate`.

## Code lists

`code_lists/myinvois.json` holds the authority's nine lists (classification,
country, currency, document type, industry, payment means, state, tax type and
unit) as read from the SDK on 12 October 2026, with each list's source URL. They
are compiled into the crate as the reference data every company starts from, so
a refreshed list is a new file in a release, reviewed like code. A company's
override (`override_code`, stored in `finance_einvoice_code_overrides`) renames a
code, retires it or adds one the file lacks, for that company only.

## Database

Migrations `finance-einvoice-001-documents`, `-002-consolidation` and
`-003-source-links` build the end state of Bilimbi's schema (b-dom-finance
`4abfaf8`), 26 triggers and 11 functions on seven tables:

- `finance_einvoice_documents`, `_document_lines`, `_submissions`,
  `_status_events`, `_consolidated_sources` and `_source_links` are
  append-only: PostgreSQL refuses UPDATE, DELETE and TRUNCATE, so a
  cancellation or a rejection is a new status event.
- The database stamps `recorded_at`, numbers a document's status events 1, 2,
  3, ..., refuses a line added to a document recorded in an earlier
  transaction, checks a submission's SHA-256 against its bytes, and refuses a
  commit in which a document has no lines or its totals differ from its lines'
  (or, for a consolidated document, its sources').
- A document's `posted` and `posting_reversed` events each occur once.
- A source is covered by one consolidated document at a time and is never
  submitted on its own while covered; a source transaction has one standing
  fiscal document.
- Rows of one company reference each other through composite
  `(id, company_id)` foreign keys.
- `finance_einvoice_code_overrides` is the one editable table.

Prove it with `fixtures/walkthrough` on a disposable database (see the
repository README); its ledger bridge section (inside the clearance section)
opens Company 002's book, retries the postings its earlier validations could
not make, posts and reverses on validation and cancellation, consolidates
September 2026 (refused, released, consolidated again), generates and reads a
PDF, links Company 001's documents to Receivables' and Payables' entries, and
pages the evidence. Its e-invoice section prints the facade's answers, the
drafts accepted and refused, the payload a recorded document becomes (unsigned
and signed, verified, and tampered) and each database refusal. Its clearance
section (last) serves a stand-in for the MyInvois API on the loopback for the
run, which checks each submission's hash and verifies its signature and
answers by the code number, then clears Company 002's documents through the
facade and the real submit and poll jobs: valid, refused, invalid, rate
limited, a duplicate found and one not found, a lost answer found again, the
72-hour overdue notice and recovery, an expired token, a rejection request, a
portal cancellation, and cancellations accepted and refused; then the stored
submissions, the database's refusals, the exchanges Base Integration recorded
(without bodies or the secret) and the audit actions.
