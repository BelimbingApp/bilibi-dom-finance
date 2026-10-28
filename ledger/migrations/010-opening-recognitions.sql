-- Opening subledgers recognize an approved opening entry; they do not post it again.
CREATE TABLE finance_ledger_opening_recognitions (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  entry_id bigint NOT NULL, account_id bigint NOT NULL,
  owner_module varchar(100) NOT NULL, owner_record varchar(100) NOT NULL,
  balance numeric(20,4) NOT NULL,
  UNIQUE (company_id, account_id),
  FOREIGN KEY (entry_id, company_id) REFERENCES finance_ledger_entries(id, company_id),
  FOREIGN KEY (account_id, company_id) REFERENCES finance_ledger_accounts(id, company_id)
);
CREATE INDEX finance_ledger_opening_recognitions_scope ON finance_ledger_opening_recognitions(tenant_id, company_id);
CREATE TRIGGER finance_ledger_opening_recognitions_kept BEFORE UPDATE OR DELETE ON finance_ledger_opening_recognitions
FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_opening_recognitions_retain BEFORE TRUNCATE ON finance_ledger_opening_recognitions
FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
-- The facade gives the field refusal; this fence also protects direct SQL.
CREATE FUNCTION finance_ledger_keep_recognized_opening() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM finance_ledger_opening_recognitions r WHERE r.tenant_id=NEW.tenant_id AND r.company_id=NEW.company_id AND r.entry_id=NEW.reverses_entry_id) THEN
  RAISE EXCEPTION 'opening items depend on this entry' USING ERRCODE='restrict_violation';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_ledger_keep_recognized_opening BEFORE INSERT ON finance_ledger_entries
FOR EACH ROW EXECUTE FUNCTION finance_ledger_keep_recognized_opening();
