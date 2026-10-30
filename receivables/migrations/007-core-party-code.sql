-- Finance keeps terms and one permanent Core party reference. No existing
-- rows are converted: walkthrough/development databases are disposable.
DROP TRIGGER finance_receivables_customers_identity ON finance_receivables_customers;
ALTER TABLE finance_receivables_customers
  DROP CONSTRAINT finance_receivables_customers_identity,
  DROP CONSTRAINT finance_receivables_customers_record_kind,
  DROP COLUMN code CASCADE, DROP COLUMN name, DROP COLUMN tax_identifier,
  DROP COLUMN registration_identifier, DROP COLUMN email, DROP COLUMN phone,
  DROP COLUMN line1, DROP COLUMN line2, DROP COLUMN line3, DROP COLUMN locality,
  DROP COLUMN postcode, DROP COLUMN admin1_code, DROP COLUMN country_iso,
  ALTER COLUMN record_kind SET NOT NULL, ALTER COLUMN record_id SET NOT NULL,
  ADD CONSTRAINT finance_receivables_customers_party_kind CHECK (record_kind IN ('company','person')),
  ADD CONSTRAINT finance_receivables_customers_party_id CHECK (record_id ~ '^[1-9][0-9]*$');
CREATE TRIGGER finance_receivables_customers_identity BEFORE UPDATE ON finance_receivables_customers
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_identity('tenant_id','company_id','record_kind','record_id');
ALTER TABLE finance_receivables_documents ADD COLUMN party_code text,
  ADD CONSTRAINT finance_receivables_documents_party_code CHECK (status <> 'issued' OR party_code IS NOT NULL);
CREATE FUNCTION finance_receivables_fixed_party_code() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.party_code IS NOT NULL AND NEW.party_code IS DISTINCT FROM OLD.party_code THEN
    RAISE EXCEPTION 'an issued document keeps its Core party code' USING ERRCODE='restrict_violation';
  END IF; RETURN NEW;
END $$;
CREATE TRIGGER finance_receivables_fixed_party_code BEFORE UPDATE ON finance_receivables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_fixed_party_code();
