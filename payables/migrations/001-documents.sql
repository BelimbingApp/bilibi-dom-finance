-- Mirrors Bilimbi Finance.Payables create schema; matching and payments are later slices.

CREATE TABLE finance_payables_settings (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  payable_account_id bigint NOT NULL,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL
);

CREATE UNIQUE INDEX finance_payables_settings_company_unique
  ON finance_payables_settings (company_id);

CREATE INDEX finance_payables_settings_tenant_id_company_id_index
  ON finance_payables_settings (tenant_id, company_id);

CREATE TABLE finance_payables_tax_codes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(50) NOT NULL,
  name varchar(200) NOT NULL,
  rate numeric(7, 4) NOT NULL,
  account_id bigint NOT NULL,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_payables_tax_codes_rate CHECK (rate >= 0)
);

CREATE UNIQUE INDEX finance_payables_tax_codes_id_company_unique
  ON finance_payables_tax_codes (id, company_id);

CREATE UNIQUE INDEX finance_payables_tax_codes_company_code_unique
  ON finance_payables_tax_codes (company_id, code);

CREATE INDEX finance_payables_tax_codes_tenant_id_company_id_index
  ON finance_payables_tax_codes (tenant_id, company_id);

CREATE TABLE finance_payables_settings_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  old_payable_account_id bigint,
  new_payable_account_id bigint NOT NULL,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL
);

CREATE INDEX finance_payables_settings_changes_tenant_id_company_id_index
  ON finance_payables_settings_changes (tenant_id, company_id);

CREATE TABLE finance_payables_tax_code_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  tax_code_id bigint NOT NULL,
  old_name varchar(200),
  new_name varchar(200) NOT NULL,
  old_rate numeric(7, 4),
  new_rate numeric(7, 4) NOT NULL,
  old_account_id bigint,
  new_account_id bigint NOT NULL,
  old_active boolean,
  new_active boolean NOT NULL,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payables_tax_code_changes_tax_code_fkey
    FOREIGN KEY (tax_code_id, company_id)
    REFERENCES finance_payables_tax_codes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_tax_code_changes_before
    CHECK ((old_name IS NULL) = (old_rate IS NULL)
      AND (old_name IS NULL) = (old_account_id IS NULL)
      AND (old_name IS NULL) = (old_active IS NULL))
);

CREATE INDEX finance_payables_tax_code_changes_tenant_id_company_id_index
  ON finance_payables_tax_code_changes (tenant_id, company_id);

CREATE TABLE finance_payables_documents (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_type varchar(16) NOT NULL,
  status varchar(16) NOT NULL,
  document_number bigint,
  corrects_document_id bigint,
  party_key varchar(100) NOT NULL,
  party_name varchar(200) NOT NULL,
  party_tax_id varchar(50),
  party_registration_id varchar(50),
  party_address varchar(1000),
  party_document_number varchar(100),
  document_date date NOT NULL,
  due_date date NOT NULL,
  currency_code varchar(3) NOT NULL,
  description varchar(500),
  total_net numeric(20, 4) NOT NULL,
  total_tax numeric(20, 4) NOT NULL,
  total numeric(20, 4) NOT NULL,
  created_by_user_id bigint NOT NULL,
  created_by_impersonator_id bigint,
  created_at timestamp(6) NOT NULL,
  approved_by_user_id bigint,
  approved_by_impersonator_id bigint,
  approved_at timestamp(6),
  posted_by_user_id bigint,
  posted_by_impersonator_id bigint,
  posted_at timestamp(6),
  payable_account_id bigint,
  entry_id bigint,
  entry_number bigint,
  voided_by_user_id bigint,
  voided_by_impersonator_id bigint,
  voided_at timestamp(6),
  void_reason varchar(500),
  CONSTRAINT finance_payables_documents_document_type
    CHECK (document_type IN ('bill', 'debit_note')),
  CONSTRAINT finance_payables_documents_status
    CHECK (status IN ('draft', 'approved', 'posted', 'voided')),
  CONSTRAINT finance_payables_documents_currency_code
    CHECK (currency_code ~ '^[A-Z]{3}$'),
  CONSTRAINT finance_payables_documents_correction
    CHECK ((document_type = 'debit_note') = (corrects_document_id IS NOT NULL)),
  CONSTRAINT finance_payables_documents_party_document_number
    CHECK (document_type <> 'bill' OR party_document_number IS NOT NULL),
  CONSTRAINT finance_payables_documents_due_date CHECK (due_date >= document_date),
  CONSTRAINT finance_payables_documents_totals
    CHECK (total_net > 0 AND total_tax >= 0 AND total = total_net + total_tax),
  CONSTRAINT finance_payables_documents_approval
    CHECK ((approved_at IS NULL) = (approved_by_user_id IS NULL)
      AND (status NOT IN ('approved', 'posted') OR approved_at IS NOT NULL)
      AND (status <> 'draft' OR approved_at IS NULL)),
  CONSTRAINT finance_payables_documents_posting
    CHECK ((status = 'posted') = (posted_at IS NOT NULL)
      AND (status = 'posted') = (document_number IS NOT NULL)
      AND (status = 'posted') = (posted_by_user_id IS NOT NULL)
      AND (status = 'posted') = (payable_account_id IS NOT NULL)
      AND (status = 'posted') = (entry_id IS NOT NULL)
      AND (status = 'posted') = (entry_number IS NOT NULL)),
  CONSTRAINT finance_payables_documents_void
    CHECK ((status = 'voided') = (voided_at IS NOT NULL)
      AND (status = 'voided') = (voided_by_user_id IS NOT NULL)
      AND (status = 'voided') = (void_reason IS NOT NULL)),
  CONSTRAINT finance_payables_documents_document_number CHECK (document_number > 0)
);

CREATE UNIQUE INDEX finance_payables_documents_id_company_unique
  ON finance_payables_documents (id, company_id);

ALTER TABLE finance_payables_documents
  ADD CONSTRAINT finance_payables_documents_corrects_document_fkey
  FOREIGN KEY (corrects_document_id, company_id)
  REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT;

CREATE UNIQUE INDEX finance_payables_documents_company_number_unique
  ON finance_payables_documents (company_id, document_type, document_number)
  WHERE document_number IS NOT NULL;

CREATE UNIQUE INDEX finance_payables_documents_party_document_unique
  ON finance_payables_documents (company_id, party_key, party_document_number)
  WHERE document_type = 'bill' AND status <> 'voided';

CREATE INDEX finance_payables_documents_company_date_index
  ON finance_payables_documents (tenant_id, company_id, document_date);

CREATE INDEX finance_payables_documents_corrects_document_id_index
  ON finance_payables_documents (corrects_document_id)
  WHERE corrects_document_id IS NOT NULL;

CREATE TABLE finance_payables_document_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  line_number integer NOT NULL,
  description varchar(500) NOT NULL,
  account_id bigint NOT NULL,
  amount numeric(20, 4) NOT NULL,
  tax_code_id bigint,
  external_order_line_id varchar(100),
  CONSTRAINT finance_payables_document_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_document_lines_tax_code_fkey
    FOREIGN KEY (tax_code_id, company_id)
    REFERENCES finance_payables_tax_codes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_document_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_payables_document_lines_amount CHECK (amount > 0)
);

CREATE UNIQUE INDEX finance_payables_document_lines_document_line_unique
  ON finance_payables_document_lines (document_id, line_number);

CREATE TABLE finance_payables_document_tax_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  line_number integer NOT NULL,
  tax_code_id bigint NOT NULL,
  tax_code varchar(50) NOT NULL,
  rate numeric(7, 4) NOT NULL,
  account_id bigint NOT NULL,
  taxable_amount numeric(20, 4) NOT NULL,
  tax_amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_payables_document_tax_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_document_tax_lines_tax_code_fkey
    FOREIGN KEY (tax_code_id, company_id)
    REFERENCES finance_payables_tax_codes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_document_tax_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_payables_document_tax_lines_amounts
    CHECK (rate >= 0 AND taxable_amount > 0 AND tax_amount >= 0)
);

CREATE UNIQUE INDEX finance_payables_document_tax_lines_document_line_unique
  ON finance_payables_document_tax_lines (document_id, line_number);

CREATE UNIQUE INDEX finance_payables_document_tax_lines_document_tax_code_unique
  ON finance_payables_document_tax_lines (document_id, tax_code_id);

CREATE FUNCTION finance_payables_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance payables keeps % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE FUNCTION finance_payables_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR (TG_TABLE_NAME = 'finance_payables_tax_codes'
        AND to_jsonb(NEW) ->> 'code' <> to_jsonb(OLD) ->> 'code') THEN
    RAISE EXCEPTION 'finance payables keeps % rows in their company and code', TG_TABLE_NAME
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_stamp_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_document_recorded() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.status <> 'draft' THEN
    RAISE EXCEPTION 'a payables document is recorded as a draft'
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.created_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_document_transition() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  stamp timestamp := clock_timestamp() AT TIME ZONE 'UTC';
BEGIN
  IF NOT ((OLD.status = 'draft' AND NEW.status IN ('approved', 'voided'))
          OR (OLD.status = 'approved' AND NEW.status IN ('posted', 'voided'))) THEN
    RAISE EXCEPTION 'payables document % cannot go from % to %; post a debit note',
      OLD.id, OLD.status, NEW.status
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF ROW(NEW.tenant_id, NEW.company_id, NEW.document_type, NEW.corrects_document_id,
         NEW.party_key, NEW.party_name, NEW.party_tax_id, NEW.party_registration_id,
         NEW.party_address, NEW.party_document_number, NEW.document_date, NEW.due_date,
         NEW.currency_code, NEW.description, NEW.total_net, NEW.total_tax, NEW.total,
         NEW.created_by_user_id, NEW.created_by_impersonator_id, NEW.created_at)
       IS DISTINCT FROM
     ROW(OLD.tenant_id, OLD.company_id, OLD.document_type, OLD.corrects_document_id,
         OLD.party_key, OLD.party_name, OLD.party_tax_id, OLD.party_registration_id,
         OLD.party_address, OLD.party_document_number, OLD.document_date, OLD.due_date,
         OLD.currency_code, OLD.description, OLD.total_net, OLD.total_tax, OLD.total,
         OLD.created_by_user_id, OLD.created_by_impersonator_id, OLD.created_at)
    OR (NEW.status <> 'approved'
        AND ROW(NEW.approved_by_user_id, NEW.approved_by_impersonator_id)
            IS DISTINCT FROM ROW(OLD.approved_by_user_id, OLD.approved_by_impersonator_id))
    OR (NEW.status <> 'posted'
        AND ROW(NEW.posted_by_user_id, NEW.posted_by_impersonator_id,
                NEW.payable_account_id, NEW.entry_id, NEW.entry_number)
            IS DISTINCT FROM
            ROW(OLD.posted_by_user_id, OLD.posted_by_impersonator_id,
                OLD.payable_account_id, OLD.entry_id, OLD.entry_number))
    OR (NEW.status <> 'voided'
        AND ROW(NEW.voided_by_user_id, NEW.voided_by_impersonator_id, NEW.void_reason)
            IS DISTINCT FROM
            ROW(OLD.voided_by_user_id, OLD.voided_by_impersonator_id, OLD.void_reason)) THEN
    RAISE EXCEPTION 'payables document % keeps what was recorded; only its status moves',
      OLD.id
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.approved_at := OLD.approved_at;
  NEW.posted_at := OLD.posted_at;
  NEW.document_number := OLD.document_number;
  NEW.voided_at := OLD.voided_at;

  IF NEW.status = 'approved' THEN
    NEW.approved_at := stamp;
  ELSIF NEW.status = 'posted' THEN
    NEW.posted_at := stamp;
    NEW.document_number := COALESCE(
      (SELECT max(document_number) FROM finance_payables_documents
       WHERE company_id = NEW.company_id AND document_type = NEW.document_type), 0) + 1;
  ELSE
    NEW.voided_at := stamp;
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_refuse_closed_line() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_payables_documents
    WHERE id = NEW.document_id
      AND (status <> 'draft' OR NOT (pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress'))
  ) THEN
    RAISE EXCEPTION 'payables document % is already recorded: a line cannot be added',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_check_document() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
  document record;
BEGIN
  IF TG_TABLE_NAME = 'finance_payables_documents' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.document_id;
  END IF;

  SELECT total_net, total_tax INTO document
  FROM finance_payables_documents WHERE id = checked_id;

  IF NOT EXISTS (
    SELECT 1 FROM finance_payables_document_lines WHERE document_id = checked_id
  ) THEN
    RAISE EXCEPTION 'payables document % has no lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF document.total_net <> (SELECT sum(amount) FROM finance_payables_document_lines
                            WHERE document_id = checked_id)
    OR document.total_tax <> (SELECT COALESCE(sum(tax_amount), 0)
                              FROM finance_payables_document_tax_lines
                              WHERE document_id = checked_id) THEN
    RAISE EXCEPTION 'payables document % totals differ from its lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_payables_document_tax_lines t
    WHERE t.document_id = checked_id
      AND t.taxable_amount <> (
        SELECT COALESCE(sum(l.amount), 0) FROM finance_payables_document_lines l
        WHERE l.document_id = checked_id AND l.tax_code_id = t.tax_code_id)
  ) OR EXISTS (
    SELECT 1 FROM finance_payables_document_lines l
    WHERE l.document_id = checked_id AND l.tax_code_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1 FROM finance_payables_document_tax_lines t
        WHERE t.document_id = checked_id AND t.tax_code_id = l.tax_code_id)
  ) THEN
    RAISE EXCEPTION 'payables document % tax lines differ from its lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

CREATE TRIGGER finance_payables_documents_recorded
  BEFORE INSERT ON finance_payables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_payables_document_recorded();

CREATE TRIGGER finance_payables_documents_transition
  BEFORE UPDATE ON finance_payables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_payables_document_transition();

CREATE TRIGGER finance_payables_settings_kept BEFORE DELETE ON finance_payables_settings FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_settings_no_truncate BEFORE TRUNCATE ON finance_payables_settings FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_tax_codes_kept BEFORE DELETE ON finance_payables_tax_codes FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_tax_codes_no_truncate BEFORE TRUNCATE ON finance_payables_tax_codes FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_settings_changes_kept BEFORE UPDATE OR DELETE ON finance_payables_settings_changes FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_settings_changes_no_truncate BEFORE TRUNCATE ON finance_payables_settings_changes FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_tax_code_changes_kept BEFORE UPDATE OR DELETE ON finance_payables_tax_code_changes FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_tax_code_changes_no_truncate BEFORE TRUNCATE ON finance_payables_tax_code_changes FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_documents_kept BEFORE DELETE ON finance_payables_documents FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_documents_no_truncate BEFORE TRUNCATE ON finance_payables_documents FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_document_lines_kept BEFORE UPDATE OR DELETE ON finance_payables_document_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_document_lines_no_truncate BEFORE TRUNCATE ON finance_payables_document_lines FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_document_tax_lines_kept BEFORE UPDATE OR DELETE ON finance_payables_document_tax_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_document_tax_lines_no_truncate BEFORE TRUNCATE ON finance_payables_document_tax_lines FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_settings_identity BEFORE UPDATE ON finance_payables_settings FOR EACH ROW EXECUTE FUNCTION finance_payables_keep_identity();

CREATE TRIGGER finance_payables_tax_codes_identity BEFORE UPDATE ON finance_payables_tax_codes FOR EACH ROW EXECUTE FUNCTION finance_payables_keep_identity();

CREATE TRIGGER finance_payables_settings_changes_stamp BEFORE INSERT ON finance_payables_settings_changes FOR EACH ROW EXECUTE FUNCTION finance_payables_stamp_change();

CREATE TRIGGER finance_payables_tax_code_changes_stamp BEFORE INSERT ON finance_payables_tax_code_changes FOR EACH ROW EXECUTE FUNCTION finance_payables_stamp_change();

CREATE TRIGGER finance_payables_document_lines_open_document BEFORE INSERT ON finance_payables_document_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_closed_line();

CREATE TRIGGER finance_payables_document_tax_lines_open_document BEFORE INSERT ON finance_payables_document_tax_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_closed_line();

CREATE CONSTRAINT TRIGGER finance_payables_documents_complete AFTER INSERT ON finance_payables_documents DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_document();

CREATE CONSTRAINT TRIGGER finance_payables_document_lines_complete AFTER INSERT ON finance_payables_document_lines DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_document();

CREATE CONSTRAINT TRIGGER finance_payables_document_tax_lines_complete AFTER INSERT ON finance_payables_document_tax_lines DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_document();

ALTER TABLE finance_payables_settings ADD COLUMN quantity_tolerance_kind varchar(16), ADD COLUMN quantity_tolerance numeric(20,4), ADD CHECK ((quantity_tolerance_kind IS NULL) = (quantity_tolerance IS NULL) AND (quantity_tolerance_kind IS NULL OR quantity_tolerance_kind IN ('percent','amount')) AND (quantity_tolerance IS NULL OR quantity_tolerance >= 0));

ALTER TABLE finance_payables_settings ADD COLUMN price_tolerance_kind varchar(16), ADD COLUMN price_tolerance numeric(20,4), ADD CHECK ((price_tolerance_kind IS NULL) = (price_tolerance IS NULL) AND (price_tolerance_kind IS NULL OR price_tolerance_kind IN ('percent','amount')) AND (price_tolerance IS NULL OR price_tolerance >= 0));

ALTER TABLE finance_payables_settings_changes ADD COLUMN old_quantity_tolerance_kind varchar(16), ADD COLUMN old_quantity_tolerance numeric(20,4), ADD CHECK ((old_quantity_tolerance_kind IS NULL) = (old_quantity_tolerance IS NULL) AND (old_quantity_tolerance_kind IS NULL OR old_quantity_tolerance_kind IN ('percent','amount')) AND (old_quantity_tolerance IS NULL OR old_quantity_tolerance >= 0));

ALTER TABLE finance_payables_settings_changes ADD COLUMN old_price_tolerance_kind varchar(16), ADD COLUMN old_price_tolerance numeric(20,4), ADD CHECK ((old_price_tolerance_kind IS NULL) = (old_price_tolerance IS NULL) AND (old_price_tolerance_kind IS NULL OR old_price_tolerance_kind IN ('percent','amount')) AND (old_price_tolerance IS NULL OR old_price_tolerance >= 0));

ALTER TABLE finance_payables_settings_changes ADD COLUMN new_quantity_tolerance_kind varchar(16), ADD COLUMN new_quantity_tolerance numeric(20,4), ADD CHECK ((new_quantity_tolerance_kind IS NULL) = (new_quantity_tolerance IS NULL) AND (new_quantity_tolerance_kind IS NULL OR new_quantity_tolerance_kind IN ('percent','amount')) AND (new_quantity_tolerance IS NULL OR new_quantity_tolerance >= 0));

ALTER TABLE finance_payables_settings_changes ADD COLUMN new_price_tolerance_kind varchar(16), ADD COLUMN new_price_tolerance numeric(20,4), ADD CHECK ((new_price_tolerance_kind IS NULL) = (new_price_tolerance IS NULL) AND (new_price_tolerance_kind IS NULL OR new_price_tolerance_kind IN ('percent','amount')) AND (new_price_tolerance IS NULL OR new_price_tolerance >= 0));

ALTER TABLE finance_payables_document_lines
  ADD COLUMN order_line_id bigint,
  ADD COLUMN quantity numeric(20,4),
  ADD CHECK ((order_line_id IS NULL) = (quantity IS NULL) AND (quantity IS NULL OR quantity > 0));
