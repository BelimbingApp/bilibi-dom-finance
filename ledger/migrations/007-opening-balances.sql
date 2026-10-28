-- Opening-balance source evidence, linked to the ordinary approval journal.
-- This slice creates no operational subledger items or historical transactions.
CREATE TABLE finance_ledger_opening_balances (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  source_reference varchar(100) NOT NULL CHECK (btrim(source_reference) <> ''),
  source_csv text NOT NULL CHECK (octet_length(source_csv) <= 131072),
  journal_id bigint NOT NULL,
  UNIQUE (company_id, source_reference),
  UNIQUE (journal_id),
  FOREIGN KEY (journal_id, company_id)
    REFERENCES finance_ledger_journals (id, company_id) ON DELETE RESTRICT
);
CREATE INDEX finance_ledger_opening_balances_tenant_company
  ON finance_ledger_opening_balances (tenant_id, company_id);
CREATE TRIGGER finance_ledger_opening_balances_append_only
  BEFORE UPDATE OR DELETE ON finance_ledger_opening_balances
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_opening_balances_no_truncate
  BEFORE TRUNCATE ON finance_ledger_opening_balances
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
