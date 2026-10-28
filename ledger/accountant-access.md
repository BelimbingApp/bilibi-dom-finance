# Accountant access

An External accountant can read and export the complete ledger and reports of a company they work in (their current company), and submit adjusting journals for an authorized company reviewer.

This is part of [Finance #61](https://github.com/BelimbingApp/bilibi-dom-finance/issues/61). It implements the [practitioner's draft/submit recommendation](https://github.com/BelimbingApp/bilibi-dom-finance/issues/61#issuecomment-6461497724), not the unresolved sales-only restriction. The agreed boundary permits posted payroll journal detail and denies the Payroll module and its per-employee source records.

## Install and assign

1. Mount Finance with Ledger and Reports, migrate normally and run `cargo run -p bilibi-web -- authz-reconcile`. Reconciliation installs the `external_accountant` system role in each tenant; deploying code does not assign it.
2. Bind the named person's account to the intended company's live affiliation through Core account administration. Assign **External accountant** through Authorization. The grant is audited and subject to the Platform's grant-administration checks and 2FA policy.
3. Review the person's other roles and direct grants. **This role adds permissions; it is not a deny policy.** Remove unrelated grants, especially tenant-wide company reach, approval, posting, settings and Payroll access. Direct denies override role grants.
4. Revoke the assignment when the engagement ends. Engagement purpose and dates must currently be managed outside this role; the Platform does not enforce an engagement period.

The role contains exactly `finance.ledger.view`, `finance.ledger.journals.submit` and `finance.reports.reports-export`. Ledger and Reports contribute only their own keys, through the shared Authz role vocabulary. No role-name checks or alternate authorization path exist in Finance.

## What the role permits

| Responsibility | Result |
| --- | --- |
| Book, chart, entries, manual journals, account totals, trial balance | Complete company ledger, including references, descriptions, payroll postings and mixed entries. No account, customer or line isolation. |
| Management reports and drilldowns | Complete figures at the usual shared entry pin. Reports remain management reports, not statutory financial statements. |
| Adjusting journals | Submit balanced manual lines for approval. No direct posting, rejection or approval. A separately authorized company reviewer approves through the existing duty-separation policy. The role cannot change that policy. |
| Auditor-pack button | Ledger-and-reports ZIP: general ledger, all Ledger evidence datasets, trial balance and issued reports. The page and manifest identify excluded sources. Payroll journal lines are included; Payroll sources are excluded. |
| Payroll, receivables/payables, reconciliation, tax, fixed assets, attachments | No source-module permission from this role. These modules' own records, source evidence and downloads still require their owner's grants. |
| Chart/settings changes, closing/reopening, control changes, report issuance | No permission from this role. |
| Another company or tenant | Refused. The role grants no tenant-wide reach or cross-tenant access. |

A company can authorize several accountants. The current Platform binds an account to one active affiliation; it does not attach this role assignment to a selectable set of company engagements. Granting tenant-wide company reach would expand every held capability to other companies and is unsuitable as a substitute. [Platform #203](https://github.com/BelimbingApp/bilibi/issues/203) tracks explicit company context; its selector must preserve owner authorization and cannot itself grant access.

## Exports and revocation

The export facade chooses coverage using current permissions, independently of role names. Anyone holding ledger view and export can generate a ledger-and-reports pack. Only a person holding every Finance source view can generate the full pack with subledger evidence, aging, attachment indexes and authority payloads. Missing one source permission excludes all source evidence; it does not silently omit ledger entries.

The manifest records `coverage`, the omitted modules, the company, person, time, snapshot, entry cutoff, totals and file hashes. Payroll sources are named explicitly in the restricted coverage notice. The restricted pack is not a complete audit evidence pack. All ledger rows, including mixed payroll/sales entries, remain readable and exportable; totals therefore remain complete.

The artifact's immutable reference stores its coverage. Publication and every subsequent download recheck that coverage's permissions and company. Losing any source permission blocks an old full pack; it cannot be relabelled as restricted. Losing ledger view or export blocks both pack kinds, including existing links. Base Artifacts records publication and reads; Authz records role changes, direct denies and authorization decisions. There is no new export-purpose prompt or scheduled engagement expiry in this change.

The current Platform dashboard shows tenant-wide company counts to signed-in users, even without Company view. It does not grant access to those companies or books. Withholding those counts, if required for an engagement, is a Platform follow-up; Finance does not change the shared dashboard.

## Every door uses the same authority

UI routes, retained live events, operation reads, MCP and journal preview/commit call the existing owner facades with a sealed scope and explicit company target. The facade rechecks writes inside the audited mutation transaction and company lock. An MCP plan cannot grant approval or change its company after review. Revocation before commit must refuse the reviewed write and leave no journal behind.

Attachments reauthorize their source on access, rather than trusting a page flag or a copied URL. No new source-document or attachment grant is added. Payroll source export remains protected by Payroll; restricted report packs never invoke those source evidence readers. The full artifact retains its original source requirements.

Posting notices carry identifiers, not payroll or sales details; consumers must read again through the authorized facade. A previously delivered notice or browser data cannot be recalled. Revocation blocks subsequent requests, reads, downloads and writes; it does not erase data already legitimately exported. Finance does not introduce a new search or notification projection in this change. Any future projection must use the owner's policy at query/delivery time.

The existing MCP operation reads return ledger, chart, journal and trial-balance data. Management-report figures and Payroll source records have no MCP read contract in this revision; this change does not add one. Journal `plan` and `commit` use the same permissions and independent approval as the UI. Auditor-pack generation remains the Reports facade/page action; there is no MCP pack-generation contract yet.

## What remains open

**Client-specific advice:** confirm the engagement, lawful access, confidentiality, retention, payroll source access and any independent auditor's appointment with the company's practitioners. An External accountant is not an independent-auditor role. The practitioner's legal references are context, not new statutory access implemented by Finance.

Sales-only access still requires a decision on workflow, account, transaction, customer and source-document boundaries. Existing field masks and tenant-wide capabilities do not provide record isolation. Mixed entries, indirect disclosures through totals, account labels, journals, reports, exports, search and notifications must be addressed together. Restricted totals must never be presented as complete financial statements. Supervisors and duly appointed auditors need explicit oversight grants, not a role-name bypass. Optional posting delegation, bounded adjustment accounts/periods, engagement metadata/expiry, export purpose and fine-grained source evidence remain follow-ups to #61 and Platform context work.

## Running proof

Run the `auditor-pack` walkthrough on a fresh disposable PostgreSQL 18 database with `AUDITOR_PACK_PROOF_DIR` naming a private proof directory and `APP_ORIGIN` matching the running app. It first proves the full pack, then assigns the production role to two affiliated accountant identities. It exercises permitted journal preparation and independent approval, denied direct posting/settings/Payroll/source access, other-company and tenant targeting, complete mixed entries, reports, scoped ZIP membership and role/direct-deny revocation against retained scopes and old links.

Append `-- serve` to use the same database in the browser. `viewer@example.test` and `staff@example.test` are the external accountants; `manager@example.test` is the company reviewer. All use the walkthrough password. The fixture saves OAuth credentials privately for an HTTP MCP proof; it never prints them. Its `deny-journals`/`restore-journals` and `deny-ledger`/`restore-ledger` modes change the first accountant's grants through Authz for revocation between plan and commit.

With that server running, run `BILIBI_AGENT_PROOF=disposable BILIBI_ACCOUNTANT_CONTROL=/absolute/path/to/target/debug/auditor-pack python3 apps/domains/finance/fixtures/walkthrough/external-accountant.py`. Keep `APP_ORIGIN`, `AUDITOR_PACK_PROOF_DIR`, `DATABASE_URL` (for the control binary) and libpq's `PGHOST`, `PGPORT`, `PGUSER`, `PGDATABASE` set to the same disposable instance. The script uses actual OAuth-authenticated `/mcp` requests for both people, reviews/commits journals, proves independent approval and other-company/source/settings refusals, and revokes submit authority between preview and commit. It observes rollback with SQL; no passwords or tokens are printed.
