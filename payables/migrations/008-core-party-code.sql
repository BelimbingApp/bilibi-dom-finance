-- Supplier codes are the Core party's fixed codes; Finance stores no copy.
ALTER TABLE finance_payables_vendors DROP COLUMN code CASCADE,
  DROP CONSTRAINT finance_payables_vendors_record_kind_check,
  ADD CONSTRAINT finance_payables_vendors_party_kind CHECK (record_kind IN ('company','person'));
CREATE OR REPLACE FUNCTION finance_payables_vendor_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP IN ('DELETE','TRUNCATE') THEN RAISE EXCEPTION 'vendor records cannot be deleted'; END IF;
  IF (NEW.tenant_id,NEW.company_id,NEW.record_kind,NEW.record_id,NEW.inserted_at)
      IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.record_kind,OLD.record_id,OLD.inserted_at)
    THEN RAISE EXCEPTION 'vendor identity cannot change'; END IF;
  RETURN NEW;
END $$;
ALTER TABLE finance_payables_documents ADD COLUMN party_code text;
CREATE FUNCTION finance_payables_fixed_party_code() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.party_code IS DISTINCT FROM OLD.party_code THEN
    RAISE EXCEPTION 'a recorded document keeps its Core party code' USING ERRCODE='restrict_violation';
  END IF; RETURN NEW;
END $$;
CREATE TRIGGER finance_payables_fixed_party_code BEFORE UPDATE ON finance_payables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_payables_fixed_party_code();
