-- Recognition remains permanent and continues to guard the opening journal.
ALTER TABLE finance_ledger_opening_recognitions DROP CONSTRAINT finance_ledger_opening_recognitions_company_id_account_id_key;
ALTER TABLE finance_ledger_opening_recognitions ADD UNIQUE(company_id,account_id,owner_module,owner_record);
CREATE TABLE finance_ledger_opening_recognition_replacements (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 recognition_id bigint NOT NULL UNIQUE REFERENCES finance_ledger_opening_recognitions(id),
 replaces_recognition_id bigint NOT NULL UNIQUE REFERENCES finance_ledger_opening_recognitions(id),
 CHECK(replaces_recognition_id<recognition_id)
);
CREATE TRIGGER finance_ledger_opening_recognition_replacements_kept BEFORE UPDATE OR DELETE ON finance_ledger_opening_recognition_replacements FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_opening_recognition_replacements_retain BEFORE TRUNCATE ON finance_ledger_opening_recognition_replacements FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE FUNCTION finance_ledger_opening_recognition_replacement_shape() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(
  SELECT 1 FROM finance_ledger_opening_recognitions previous JOIN finance_ledger_opening_recognitions current
   ON current.tenant_id=previous.tenant_id AND current.company_id=previous.company_id
   AND current.entry_id=previous.entry_id AND current.account_id=previous.account_id
   AND current.owner_module=previous.owner_module AND current.balance=previous.balance
  WHERE previous.id=NEW.replaces_recognition_id AND current.id=NEW.recognition_id
   AND previous.tenant_id=NEW.tenant_id AND previous.company_id=NEW.company_id
 ) THEN RAISE EXCEPTION 'opening recognition replacement must retain the owner module and approved balance' USING ERRCODE='check_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_ledger_opening_recognition_replacement_shape BEFORE INSERT ON finance_ledger_opening_recognition_replacements FOR EACH ROW EXECUTE FUNCTION finance_ledger_opening_recognition_replacement_shape();
