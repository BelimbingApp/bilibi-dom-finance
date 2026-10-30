-- The Journals list pages both record kinds by their recorded time and id.
CREATE INDEX finance_ledger_entries_journal_list_index
  ON finance_ledger_entries (tenant_id, company_id, posted_at, id);
CREATE INDEX finance_ledger_journals_journal_list_index
  ON finance_ledger_journals (tenant_id, company_id, submitted_at, id);
