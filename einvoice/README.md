# E-invoice

Module ID: `finance/einvoice` (Bilimbi `Bilimbi.Finance.Einvoice`). The fiscal
documents a company issues through a clearance authority. This version holds
the schema, the company's setup (settings, credentials, signing certificate
with its facts, and supplier profile), the `Authority` contract with the one
authority this release clears documents with (MyInvois) and its rules: the
draft a caller asks to issue and its validation, the UBL 2.1 JSON builder with
its byte-exact writer, the JSON XAdES signer, and the authority's code lists
with each company's overrides. Issuing, submitting, posting, consolidating and
the pages follow in later versions of this module (slices NE3 to NE6). Other
modules and Domains never read its tables.

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

## Capabilities

Five keys, verbatim from Bilimbi, under the `finance` domain, all granted to
`tenant_owner`: `finance.einvoice.view`, `.documents.issue`,
`.documents.submit`, `.documents.cancel` and `.settings.manage`. The module
declares the verbs `issue`, `submit` and `cancel` (Bilibi merges verbs across
modules). It has no pages, menu items or agent operations yet.

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
  without a path, defaulting to the authority's published hosts.

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
and the SDK as read on 12 October 2026). The API callbacks join the trait with
the MyInvois client, and the consolidated document's shape and refusal with
consolidation, in later versions.

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
repository README); its e-invoice section prints the facade's answers, the
drafts accepted and refused, the payload a recorded document becomes (unsigned
and signed, verified, and tampered) and each database refusal.
