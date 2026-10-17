# E-invoice

Module ID: `finance/einvoice` (Bilimbi `Bilimbi.Finance.Einvoice`). The fiscal
documents a company issues through a clearance authority. This version holds
the schema, the company's setup (settings, credentials, signing certificate and
supplier profile), the `Authority` contract with the one authority this release
clears documents with (MyInvois), and the authority's code lists with each
company's overrides. Issuing, signing, submitting, posting, consolidating and
the pages follow in later versions of this module (slices NE2 to NE6). Other
modules and Domains never read its tables.

## Public API

Resolve the `Einvoice` service (`finance/einvoice.einvoice`) from the registry
and call it with a sealed `Scope` and an explicit company id.

| Function | Capability | Does |
| --- | --- | --- |
| `Einvoice::code_lists` | none | The lists the authority publishes, sorted |
| `list_codes` | `finance.einvoice.view` | The company's active codes in one list, code to description: the shipped list with the company's overrides applied; `UnknownList` for a list the authority lacks |
| `override_code` | `finance.einvoice.settings.manage` | Renames a code, names one the shipped list lacks, or retires it, for that company only; audit action `finance.einvoice.code.overridden` |
| `setup` | `finance.einvoice.settings.manage` | What the company has configured, never a secret (see Settings) |
| `Einvoice::authority` | none | The `Authority` every company's documents are cleared with |

Every call authorizes its capability for the scope's actor and the target
company at the moment it runs (Core Company's `authorize_target`): a missing
grant, or a sibling company without `admin.company.tenant-wide.manage`, is
`EinvoiceError::Unauthorized`; a company the scope cannot see is
`CompanyNotFound`. A write then holds the company's row lock inside an audited
`MutationBoundary` transaction, so an archived company is `CompanyArchived`.
Refused values are `Invalid(Vec<FieldError>)` with Bilimbi's changeset words
(`must be 1 to 16 letters, digits, dots, or dashes`, `should be at most 500
character(s)`).

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
  validated as one readable X.509 certificate and one unencrypted RSA private
  key (`PRIVATE KEY` or `RSA PRIVATE KEY`; PKCS#12, encrypted and EC keys are
  refused).
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
on screen and revealed only through its password-checked reveal. `setup` reads
them masked: it reports only whether a client secret, certificate and private
key are set.

## Authority

`Authority` is everything one clearance authority defines that needs no
network: its document types, posting rules, format shapes, versions, the
consolidation window and the validation link. MyInvois is the only
implementation (Guideline v4.8 and the SDK as read on 12 October 2026). The
callbacks that take a draft or a recorded document, and the API callbacks, join
the trait with the builder, signer and client in later versions, each with its
own types.

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
repository README); its e-invoice section prints the facade's answers and each
refusal.
