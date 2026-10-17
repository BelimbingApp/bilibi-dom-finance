# Attachments

Module ID: `finance/attachments` (Bilimbi `Bilimbi.Finance.Attachments`).
Source documents kept with the Finance records they support: a supplier's
invoice scan behind a bill, a remittance behind a payment, a contract behind an
invoice, a voucher behind a manual journal, the bank's own file behind an
imported statement. This version is the services; the Source documents page,
the download route and the embeddable panel follow in the UI slice, and the
manual-journal source follows the ledger's journals.

Depends on `finance/ledger` (the financial year a document's retention counts
from is the ledger's), Core Company, Base Artifacts, Base Queue and Base
Tenancy. Record owners (Receivables, Payables, Reconciliation, ...) depend on
this module, never the other way round.

## Public API

Resolve the `Attachments` service (`finance/attachments.attachments`) and call
it with a sealed `Scope` and an explicit company id. Results are plain records
(`Attachment`, `Retention`, `Capture`); the schema is private.

| Function | Capability | Does |
| --- | --- | --- |
| `get_retention` | `view` | The company's retention years (7 until it lengthens them) and the latest change |
| `set_retention` | `retention.manage` | Lengthens the years (1 to 100) with a reason; fewer or equal is `RetentionNotLengthened` |
| `list_retention_changes` | `view` | Every change, oldest first |
| `retain_until` | `view` + `finance.ledger.view` | What a document dated `date` would be kept until |
| `attach` | `create` | Stores the file as a records-grade artifact and links it to a record |
| `capture_file` / `attach_capture` | `create` | The same in two steps, for a record inserted in the caller's own transaction (a bank statement): capture first, link inside the transaction |
| `replace` | `create` | Captures a new file and ends the old attachment with `replaced`; only while the record is not final |
| `remove` | `delete` | Ends an attachment with `removed` and a reason; only while the record is not final |
| `extend_retention` | `retention.manage` + `view` | Moves one attachment's retain-until later, with a reason |
| `place_hold` / `release_hold` | `retention.manage` + `view` | A legal hold on the bytes, with a reason |
| `refresh_retention` | none (the source's own read) | Queues the retention refresh for a record that has just become final |
| `list_attachments`, `list_company_attachments`, `list_sources`, `get_attachment`, `read_attachment` | `view` + the source's view capability | The reads; a source the caller cannot read is left out of every list |
| `evidence_page` | `view` | One ID-ordered page of the `attachment`, `attachment_event` and `retention_change` datasets for the auditor's pack |

Capabilities are `finance.attachments.view`, `.create`, `.delete` and
`.retention.manage`, verbatim from Bilimbi, all granted to `tenant_owner`. The
module declares no verb. Every call authorizes the capability for the scope's
actor and the target company when it runs; a write holds the company's row
lock in an audited `MutationBoundary` transaction. Errors are
`AttachmentError` variants with Bilimbi's refusal names (`Unauthorized`,
`CompanyNotFound`, `CompanyArchived`, `DocumentFinal`, `AttachmentNotStanding`,
`RetentionNotExtended`, `AlreadyOnHold`, ...); refused values come back as
`Invalid(Vec<FieldError>)`.

## Source: the record an attachment supports

A Finance module that keeps source documents contributes one `SourceDefinition`
per kind of record to the `finance/attachments.sources` catalog, under the id
`<its module id>/<record kind>` (`finance/payables/bill`). Its `Source` says
how a record is named (`label`), the capability that reads it
(`view_capability`) and, for a record id, its `label`, `date` and `is_final`
on the caller's connection and with the caller's scope. The source authorizes
the read itself and answers `SourceRefusal::NotFound` or `Unauthorized`. The
attachments facade stores the source id and the contributing module with each
attachment and never reads the owner's tables.

A record whose date can change while it is a draft calls
`refresh_retention` inside the transaction that finalises it.

## Retention

A file is stored once in Base Artifacts as a records-grade document, retained
until the company's retention years after the end of the financial year the
record's date falls in, at the end of that day in UTC. Base refuses to delete
the bytes before then, and a legal hold refuses it after. An attachment row is
never changed; its standing is its latest event (`replaced`, `removed`) and its
retention events (`retention_extended`, `hold_placed`, `hold_released`) say what
was done to the bytes. A replaced or removed attachment stays readable.

A change of one attachment's retention is made in Base Artifacts first, after a
short transaction under the company lock has checked it, and its event follows;
Base is the fact. `get_attachment` and `read_attachment` append any retention
event Base holds that the module does not, and a change whose event could not
be recorded is `RecordedInStorage`.

`TODO(finance/ledger periods)`: the financial year is the calendar year (31
December) until the ledger's fiscal years land; the facade then reads them, as
Bilimbi does, and a company with no fiscal year keeps 31 December.

### Retention refresh job

`refresh_retention` checks, with the caller's own scope, that the record is
readable and final, computes the retain-until its final date gives and queues
the Base Queue job `finance/attachments-retention-refresh` in the caller's
transaction. The job runs as the system principal
`finance-attachments-retention-refresh` (Bilimbi
`finance.attachments.retention_refresh`; Bilibi principal names are one
hyphenated segment), which holds `finance.attachments.retention.manage` only
where a tenant administrator granted it (`Grants::set_system`); without the
grant the job cancels as `actor_refused`. The person who finalised the record
needs no attachments grant. An archived company, an unmounted source or a
removed account cancels the job; any other failure retries.
`TODO(platform)`: Base Artifacts accepts only a user's scope, so the job reaches
it through the finaliser's live scope (proved again on every attempt) and Base's
history names that person; the module's own events name the principal.

## Tables

`finance_attachments` (one row per captured file), `finance_attachment_events`
and `finance_attachments_retention_changes`. All three are append-only
(PostgreSQL refuses UPDATE, DELETE and TRUNCATE), the database numbers a
company's attachments without gaps and stamps the capture time, refuses a
shorter retention, a second `replaced`/`removed` per attachment, a replacement
by another record's attachment and a hold without a reason.
