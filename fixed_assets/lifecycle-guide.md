# Fixed Assets lifecycle

Prepare categories, read asset schedules and post monthly depreciation through the same reviewed actions as the pages.

For example, an opening asset with cost 1,200, source accumulation 300 and nine remaining months starts with carrying value 900. Its prospective straight-line charge is 100 per month. Importing it retains the approved opening balances; the first monthly run posts only the new charge.

## Prepare and continue a register

1. Read `finance.ledger.book.get` and `finance.ledger.account.list` for the explicit `company_id`.
2. Read `finance.fixed_assets.category.list`. Use `category.save` to create a category or append a complete revision. Choose active accounts that accept postings: asset accounts for cost and accumulation, expense accounts for depreciation and losses, and an income account for gains. Cost and accumulation must differ. Every revision needs a reason. Existing assets retain their saved revision.
3. For opening assets, read `opening.context` and use `opening.import` with its CSV header. Import the complete source register against approved opening balances. See the opening guide for independent approval and exact reconciliation.
4. For a later acquisition, use `asset.acquire` with a live posted entry or supplier bill and its complete debit line. It retains the source's owner and creates no acquisition posting. Opening aggregate debits are refused here.
5. Read `asset.get` for source assumptions, projected schedule, posted charges and corrections. Read `register.get` or `schedule.get` with an `as_of` date. `register.get` can filter rows by source and status; its reconciliation still covers the whole register. `schedule.get` remains unfiltered. Scheduled depreciation is a projection; accumulation is the posted figure plus retained opening accumulation.
6. Review `run.post` with the month's first day, then commit that exact plan. It posts through month-end, including earlier scheduled depreciation still unposted. Read the charge lines and skipped acquisitions before approving. Open the fiscal year first; closed periods are refused.

## Dispose and correct

Use `asset.dispose` with an effective date, reason and proceeds in the book's minor units. Zero proceeds means a write-off. Positive proceeds need an active asset account that accepts postings. Depreciation must be current through the last completed month; a month-end disposal requires that month's charge too. The review shows cost removal, accumulation, proceeds and gain or loss.

Use `disposal.reverse` to restore an asset through a new event and the owned ledger reversal. Reverse standing disposals and later runs before `run.reverse`. A consumed month cannot be run again, even after reversal; the next month's run catches up. `acquisition.void` is available only after the source entry was reversed, and reverses this asset's remaining posted depreciation without changing other assets.

## Review and reproduce

Every write requires a reviewed plan. Review each dependent step after its prerequisites commit: a category revision, run or disposal can change what a later step would do. If a combined plan changes previously reviewed facts, the whole plan is refused and rolled back. Refresh and review again.

Save the `as_of` date, any source or status filters, and all four boundaries returned by `register.get`: `through_entry_number`, `through_asset_number`, `through_event_number` and `through_category_revision_id`. Supply them together to reproduce the selected rows and whole-register reconciliation after later activity, including backdated corrections. Show every difference; never net accounts or companies together.

All actions name `company_id` and recheck access and company status. Writes retain period controls, source ownership and row audits. Amounts finer than the book permits are refused without rounding.

## Where to go next

Read the opening guide (`finance.fixed_assets.opening`) for conversion imports, or the Fixed Assets README for the accounting rules and running proofs.
