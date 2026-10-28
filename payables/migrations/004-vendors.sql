-- Vendor identity stays in Core. A key mapping never changes old evidence.
CREATE TABLE finance_payables_vendors (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(50) NOT NULL CHECK (length(btrim(code)) > 0),
  record_kind text NOT NULL CHECK (record_kind = 'company'),
  record_id text NOT NULL CHECK (record_id ~ '^[1-9][0-9]*$'),
  payment_terms_days integer NOT NULL CHECK (payment_terms_days BETWEEN 0 AND 3650),
  payable_account_id bigint,
  expense_account_id bigint,
  tax_code_id bigint,
  active boolean NOT NULL,
  inserted_at timestamp(6) NOT NULL,
  updated_at timestamp(6) NOT NULL,
  UNIQUE (tenant_id, company_id, id),
  UNIQUE (tenant_id, company_id, code),
  UNIQUE (tenant_id, company_id, record_kind, record_id),
  FOREIGN KEY (company_id) REFERENCES finance_ledger_books (company_id),
  FOREIGN KEY (payable_account_id, company_id) REFERENCES finance_ledger_accounts (id, company_id),
  FOREIGN KEY (expense_account_id, company_id) REFERENCES finance_ledger_accounts (id, company_id),
  FOREIGN KEY (tax_code_id, company_id) REFERENCES finance_payables_tax_codes (id, company_id)
);
CREATE TABLE finance_payables_vendor_keys (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  vendor_id bigint NOT NULL,
  supplier_key varchar(100) NOT NULL CHECK (length(btrim(supplier_key)) > 0),
  reason varchar(500) NOT NULL CHECK (length(btrim(reason)) > 0),
  confirmed_by_user_id bigint NOT NULL,
  confirmed_by_impersonator_id bigint,
  inserted_at timestamp(6) NOT NULL,
  updated_at timestamp(6) NOT NULL,
  UNIQUE (tenant_id, company_id, supplier_key),
  FOREIGN KEY (tenant_id, company_id, vendor_id) REFERENCES finance_payables_vendors (tenant_id, company_id, id)
);
CREATE FUNCTION finance_payables_vendor_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP IN ('DELETE','TRUNCATE') THEN RAISE EXCEPTION 'vendor records cannot be deleted'; END IF;
  IF (NEW.tenant_id,NEW.company_id,NEW.code,NEW.record_kind,NEW.record_id,NEW.inserted_at)
      IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.code,OLD.record_kind,OLD.record_id,OLD.inserted_at)
    THEN RAISE EXCEPTION 'vendor identity cannot change'; END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_payables_vendor_guard BEFORE UPDATE OR DELETE ON finance_payables_vendors
  FOR EACH ROW EXECUTE FUNCTION finance_payables_vendor_guard();
CREATE FUNCTION finance_payables_vendor_key_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'confirmed vendor keys are immutable'; END $$;
CREATE TRIGGER finance_payables_vendor_key_guard BEFORE UPDATE OR DELETE ON finance_payables_vendor_keys
  FOR EACH ROW EXECUTE FUNCTION finance_payables_vendor_key_guard();

CREATE TRIGGER finance_payables_vendor_no_truncate BEFORE TRUNCATE ON finance_payables_vendors
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_vendor_guard();
CREATE TRIGGER finance_payables_vendor_key_no_truncate BEFORE TRUNCATE ON finance_payables_vendor_keys
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_vendor_key_guard();
