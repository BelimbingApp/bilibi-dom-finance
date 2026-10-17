-- Mirrors Bilimbi finance/receivables, 20261012214517_create_finance_receivables.
-- Fresh schema: the later 20261015150100_claim_finance_receivables_entries
-- only claims legacy ledger entries and has no schema changes to port.
-- Tenant, company and user identifiers are bigint, as in finance/ledger and
-- finance/attachments. Ledger identifiers deliberately have no foreign keys:
-- the owning facade validates them, never this module's SQL.
-- Numbering and settlement checks rely on the facade's company write lock.

CREATE TABLE finance_receivables_setups (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  receivable_account_id bigint NOT NULL, income_account_id bigint NOT NULL,
  inserted_at timestamp(0) NOT NULL, updated_at timestamp(0) NOT NULL
);
CREATE UNIQUE INDEX finance_receivables_setups_company_unique
  ON finance_receivables_setups (company_id);
CREATE INDEX finance_receivables_setups_tenant_id_company_id_index
  ON finance_receivables_setups (tenant_id, company_id);

CREATE TABLE finance_receivables_setup_changes (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  field varchar(32) NOT NULL, old_value bigint, new_value bigint NOT NULL,
  changed_by_user_id bigint NOT NULL, changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_receivables_setup_changes_field
    CHECK (field IN ('receivable_account_id', 'income_account_id'))
);
CREATE INDEX finance_receivables_setup_changes_tenant_id_company_id_index
  ON finance_receivables_setup_changes (tenant_id, company_id);

CREATE TABLE finance_receivables_series (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  code varchar(50) NOT NULL, name varchar(200) NOT NULL, kind varchar(16) NOT NULL,
  prefix varchar(20) NOT NULL, number_width smallint NOT NULL,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL, updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_receivables_series_kind
    CHECK (kind IN ('invoice', 'credit_note', 'payment')),
  CONSTRAINT finance_receivables_series_number_width CHECK (number_width BETWEEN 1 AND 12)
);
CREATE UNIQUE INDEX finance_receivables_series_id_company_unique
  ON finance_receivables_series (id, company_id);
CREATE UNIQUE INDEX finance_receivables_series_id_company_kind_unique
  ON finance_receivables_series (id, company_id, kind);
CREATE UNIQUE INDEX finance_receivables_series_company_code_unique
  ON finance_receivables_series (company_id, code);
CREATE UNIQUE INDEX finance_receivables_series_company_prefix_unique
  ON finance_receivables_series (company_id, prefix);
CREATE INDEX finance_receivables_series_tenant_id_company_id_index
  ON finance_receivables_series (tenant_id, company_id);

CREATE TABLE finance_receivables_tax_codes (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  code varchar(50) NOT NULL, name varchar(200) NOT NULL, rate numeric(9, 4) NOT NULL,
  account_id bigint NOT NULL, active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL, updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_receivables_tax_codes_rate CHECK (rate >= 0 AND rate < 1000)
);
CREATE UNIQUE INDEX finance_receivables_tax_codes_id_company_unique
  ON finance_receivables_tax_codes (id, company_id);
CREATE UNIQUE INDEX finance_receivables_tax_codes_company_code_unique
  ON finance_receivables_tax_codes (company_id, code);
CREATE INDEX finance_receivables_tax_codes_tenant_id_company_id_index
  ON finance_receivables_tax_codes (tenant_id, company_id);

CREATE TABLE finance_receivables_customers (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  code varchar(50) NOT NULL, name varchar(200) NOT NULL,
  tax_identifier varchar(50), registration_identifier varchar(50),
  email varchar(255), phone varchar(50),
  line1 varchar(255), line2 varchar(255), line3 varchar(255),
  locality varchar(100), postcode varchar(20), admin1_code varchar(20),
  country_iso varchar(2), payment_terms_days integer NOT NULL DEFAULT 0,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL, updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_receivables_customers_country_iso CHECK (country_iso ~ '^[A-Z]{2}$'),
  CONSTRAINT finance_receivables_customers_payment_terms_days
    CHECK (payment_terms_days BETWEEN 0 AND 3650)
);
CREATE UNIQUE INDEX finance_receivables_customers_id_company_unique
  ON finance_receivables_customers (id, company_id);
CREATE UNIQUE INDEX finance_receivables_customers_company_code_unique
  ON finance_receivables_customers (company_id, code);
CREATE INDEX finance_receivables_customers_tenant_id_company_id_index
  ON finance_receivables_customers (tenant_id, company_id);

CREATE TABLE finance_receivables_documents (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  kind varchar(16) NOT NULL, status varchar(16) NOT NULL,
  series_id bigint NOT NULL, customer_id bigint NOT NULL, credited_document_id bigint,
  receivable_account_id bigint,
  currency_code varchar(3) NOT NULL, issue_date date, due_date date,
  description varchar(500),
  subtotal numeric(20, 4) NOT NULL, tax_total numeric(20, 4) NOT NULL,
  total numeric(20, 4) NOT NULL,
  document_number bigint, document_reference varchar(50),
  party_name varchar(200), party_tax_identifier varchar(50),
  party_registration_identifier varchar(50), party_email varchar(255),
  party_phone varchar(50), party_line1 varchar(255), party_line2 varchar(255),
  party_line3 varchar(255), party_locality varchar(100), party_postcode varchar(20),
  party_admin1_code varchar(20), party_country_iso varchar(2),
  ledger_entry_id bigint, ledger_entry_number bigint,
  issued_by_user_id bigint, issued_by_impersonator_id bigint, issued_at timestamp(6),
  discarded_by_user_id bigint, discarded_at timestamp(6),
  inserted_at timestamp(0) NOT NULL, updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_receivables_documents_series_fkey
    FOREIGN KEY (series_id, company_id, kind)
    REFERENCES finance_receivables_series (id, company_id, kind) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_documents_customer_fkey FOREIGN KEY (customer_id, company_id)
    REFERENCES finance_receivables_customers (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_documents_kind CHECK (kind IN ('invoice', 'credit_note')),
  CONSTRAINT finance_receivables_documents_status
    CHECK (status IN ('draft', 'issued', 'discarded')),
  CONSTRAINT finance_receivables_documents_credited
    CHECK ((kind = 'credit_note') = (credited_document_id IS NOT NULL)),
  CONSTRAINT finance_receivables_documents_amounts
    CHECK (subtotal >= 0 AND tax_total >= 0 AND total = subtotal + tax_total),
  CONSTRAINT finance_receivables_documents_due_date
    CHECK (due_date IS NULL OR due_date >= issue_date),
  CONSTRAINT finance_receivables_documents_issued CHECK (
    (status = 'issued') = (document_number IS NOT NULL)
    AND (status <> 'issued' OR (
      document_number > 0 AND document_reference IS NOT NULL
      AND issue_date IS NOT NULL AND (kind <> 'invoice' OR due_date IS NOT NULL)
      AND total > 0 AND party_name IS NOT NULL AND receivable_account_id IS NOT NULL
      AND ledger_entry_id IS NOT NULL AND ledger_entry_number IS NOT NULL
      AND issued_by_user_id IS NOT NULL AND issued_at IS NOT NULL))),
  CONSTRAINT finance_receivables_documents_discarded CHECK (
    (status = 'discarded') = (discarded_at IS NOT NULL)
    AND (status <> 'discarded' OR discarded_by_user_id IS NOT NULL))
);
CREATE UNIQUE INDEX finance_receivables_documents_id_company_unique
  ON finance_receivables_documents (id, company_id);
ALTER TABLE finance_receivables_documents
  ADD CONSTRAINT finance_receivables_documents_credited_fkey
  FOREIGN KEY (credited_document_id, company_id)
  REFERENCES finance_receivables_documents (id, company_id) ON DELETE RESTRICT;
CREATE UNIQUE INDEX finance_receivables_documents_series_number_unique
  ON finance_receivables_documents (series_id, document_number)
  WHERE document_number IS NOT NULL;
CREATE UNIQUE INDEX finance_receivables_documents_company_reference_unique
  ON finance_receivables_documents (company_id, document_reference)
  WHERE document_reference IS NOT NULL;
CREATE INDEX finance_receivables_documents_customer_index
  ON finance_receivables_documents (tenant_id, company_id, customer_id);
CREATE INDEX finance_receivables_documents_credited_document_id_index
  ON finance_receivables_documents (credited_document_id);
CREATE INDEX finance_receivables_documents_company_entry_number_index
  ON finance_receivables_documents (company_id, ledger_entry_number)
  WHERE ledger_entry_number IS NOT NULL;

CREATE TABLE finance_receivables_document_lines (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  document_id bigint NOT NULL, line_number integer NOT NULL,
  description varchar(500) NOT NULL, quantity numeric(20, 4) NOT NULL,
  unit_price numeric(20, 4) NOT NULL, amount numeric(20, 4) NOT NULL,
  account_id bigint NOT NULL, tax_code_id bigint, tax_rate numeric(9, 4),
  tax_amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_receivables_document_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_receivables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_document_lines_tax_code_fkey
    FOREIGN KEY (tax_code_id, company_id)
    REFERENCES finance_receivables_tax_codes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_document_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_receivables_document_lines_amounts
    CHECK (quantity > 0 AND unit_price >= 0 AND amount >= 0 AND tax_amount >= 0),
  CONSTRAINT finance_receivables_document_lines_tax
    CHECK ((tax_code_id IS NULL) = (tax_rate IS NULL)
      AND (tax_code_id IS NOT NULL OR tax_amount = 0))
);
CREATE UNIQUE INDEX finance_receivables_document_lines_document_line_unique
  ON finance_receivables_document_lines (document_id, line_number);

CREATE TABLE finance_receivables_document_tax_lines (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  document_id bigint NOT NULL, tax_code_id bigint NOT NULL,
  tax_code varchar(50) NOT NULL, tax_rate numeric(9, 4) NOT NULL,
  taxable_amount numeric(20, 4) NOT NULL, tax_amount numeric(20, 4) NOT NULL,
  account_id bigint NOT NULL,
  CONSTRAINT finance_receivables_document_tax_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_receivables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_document_tax_lines_tax_code_fkey
    FOREIGN KEY (tax_code_id, company_id)
    REFERENCES finance_receivables_tax_codes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_document_tax_lines_amounts
    CHECK (taxable_amount >= 0 AND tax_amount >= 0)
);
CREATE UNIQUE INDEX finance_receivables_document_tax_lines_document_tax_code_unique
  ON finance_receivables_document_tax_lines (document_id, tax_code_id);

CREATE TABLE finance_receivables_payments (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  series_id bigint NOT NULL, customer_id bigint NOT NULL,
  payment_number bigint NOT NULL, payment_reference varchar(50) NOT NULL,
  payment_date date NOT NULL, amount numeric(20, 4) NOT NULL,
  currency_code varchar(3) NOT NULL, deposit_account_id bigint NOT NULL,
  reference varchar(100), description varchar(500),
  ledger_entry_id bigint NOT NULL, ledger_entry_number bigint NOT NULL,
  recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_receivables_payments_series_fkey FOREIGN KEY (series_id, company_id)
    REFERENCES finance_receivables_series (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_payments_customer_fkey FOREIGN KEY (customer_id, company_id)
    REFERENCES finance_receivables_customers (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_payments_amount CHECK (amount > 0 AND payment_number > 0)
);
CREATE UNIQUE INDEX finance_receivables_payments_id_company_unique
  ON finance_receivables_payments (id, company_id);
CREATE UNIQUE INDEX finance_receivables_payments_series_number_unique
  ON finance_receivables_payments (series_id, payment_number);
CREATE UNIQUE INDEX finance_receivables_payments_company_reference_unique
  ON finance_receivables_payments (company_id, payment_reference);
CREATE INDEX finance_receivables_payments_customer_index
  ON finance_receivables_payments (tenant_id, company_id, customer_id);

CREATE TABLE finance_receivables_allocations (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  payment_id bigint NOT NULL, document_id bigint NOT NULL, amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_receivables_allocations_payment_fkey FOREIGN KEY (payment_id, company_id)
    REFERENCES finance_receivables_payments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_allocations_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_receivables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_receivables_allocations_amount CHECK (amount > 0)
);
CREATE UNIQUE INDEX finance_receivables_allocations_payment_document_unique
  ON finance_receivables_allocations (payment_id, document_id);
CREATE INDEX finance_receivables_allocations_document_id_index
  ON finance_receivables_allocations (document_id);

CREATE TABLE finance_receivables_payment_reversals (
  id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
  payment_id bigint NOT NULL, reversal_date date NOT NULL, reason varchar(500) NOT NULL,
  ledger_entry_id bigint NOT NULL, ledger_entry_number bigint NOT NULL,
  reversed_by_user_id bigint NOT NULL, reversed_by_impersonator_id bigint,
  reversed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_receivables_payment_reversals_payment_fkey
    FOREIGN KEY (payment_id, company_id)
    REFERENCES finance_receivables_payments (id, company_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX finance_receivables_payment_reversals_payment_unique
  ON finance_receivables_payment_reversals (payment_id);
CREATE INDEX finance_receivables_payment_reversals_company_index
  ON finance_receivables_payment_reversals (tenant_id, company_id);

CREATE FUNCTION finance_receivables_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance receivables keeps % rows: % is refused',
    TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE FUNCTION finance_receivables_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  kept text;
BEGIN
  FOREACH kept IN ARRAY TG_ARGV LOOP
    IF to_jsonb(NEW) -> kept IS DISTINCT FROM to_jsonb(OLD) -> kept THEN
      RAISE EXCEPTION 'finance receivables keeps % of % rows', kept, TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
    END IF;
  END LOOP;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_outstanding(invoice_id bigint) RETURNS numeric
LANGUAGE sql STABLE AS $$
  SELECT d.total
    - COALESCE((SELECT sum(c.total) FROM finance_receivables_documents c
                WHERE c.credited_document_id = d.id AND c.status = 'issued'), 0)
    - COALESCE((SELECT sum(a.amount) FROM finance_receivables_allocations a
                WHERE a.document_id = d.id
                  AND NOT EXISTS (SELECT 1 FROM finance_receivables_payment_reversals r
                                  WHERE r.payment_id = a.payment_id)), 0)
  FROM finance_receivables_documents d
  WHERE d.id = invoice_id
$$;

CREATE FUNCTION finance_receivables_guard_document() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  credited finance_receivables_documents%ROWTYPE;
BEGIN
  IF OLD.status <> 'draft' THEN
    RAISE EXCEPTION 'finance receivables document % is %: it is never changed; issue a credit note',
      OLD.id, OLD.status
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR NEW.kind <> OLD.kind THEN
    RAISE EXCEPTION 'a finance receivables document keeps its company and kind'
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.status = 'discarded' THEN
    NEW.discarded_at := clock_timestamp() AT TIME ZONE 'UTC';
  ELSIF NEW.status = 'issued' THEN
    IF NEW.document_number IS DISTINCT FROM COALESCE(
      (SELECT max(document_number) FROM finance_receivables_documents
       WHERE series_id = NEW.series_id), 0) + 1 THEN
      RAISE EXCEPTION 'document % is not the next number in series %',
        NEW.document_number, NEW.series_id
        USING ERRCODE = 'check_violation';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM finance_receivables_document_lines WHERE document_id = NEW.id)
      OR NEW.subtotal <> (SELECT sum(amount) FROM finance_receivables_document_lines
                          WHERE document_id = NEW.id)
      OR NEW.tax_total <> (SELECT sum(tax_amount) FROM finance_receivables_document_lines
                           WHERE document_id = NEW.id)
      OR NEW.tax_total <> COALESCE((SELECT sum(tax_amount)
                                    FROM finance_receivables_document_tax_lines
                                    WHERE document_id = NEW.id), 0) THEN
      RAISE EXCEPTION 'document % totals do not equal its lines', NEW.id
        USING ERRCODE = 'check_violation';
    END IF;

    IF NEW.kind = 'credit_note' THEN
      SELECT * INTO credited FROM finance_receivables_documents
      WHERE id = NEW.credited_document_id;

      IF credited.kind <> 'invoice' OR credited.status <> 'issued'
        OR credited.customer_id <> NEW.customer_id THEN
        RAISE EXCEPTION 'a credit note credits an issued invoice of the same customer'
          USING ERRCODE = 'check_violation';
      END IF;

      IF NEW.receivable_account_id IS DISTINCT FROM credited.receivable_account_id THEN
        RAISE EXCEPTION 'a credit note posts to the receivable account of its invoice'
          USING ERRCODE = 'check_violation';
      END IF;

      IF finance_receivables_outstanding(credited.id) < NEW.total THEN
        RAISE EXCEPTION 'credit note % exceeds what invoice % still owes', NEW.id, credited.id
          USING ERRCODE = 'check_violation';
      END IF;
    END IF;

    NEW.issued_at := clock_timestamp() AT TIME ZONE 'UTC';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_insert_draft() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.status <> 'draft' THEN
    RAISE EXCEPTION 'a finance receivables document is created as a draft'
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_guard_draft_line() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  parent_id bigint;
BEGIN
  IF TG_OP = 'DELETE' THEN
    parent_id := OLD.document_id;
  ELSE
    parent_id := NEW.document_id;
  END IF;

  IF TG_OP = 'UPDATE' AND NEW.document_id <> OLD.document_id THEN
    RAISE EXCEPTION 'a line keeps its document'
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF (SELECT status FROM finance_receivables_documents WHERE id = parent_id) <> 'draft' THEN
    RAISE EXCEPTION 'finance receivables document % is no longer a draft: its lines are kept',
      parent_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF TG_OP = 'DELETE' THEN
    RETURN OLD;
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_number_payment() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF (SELECT kind FROM finance_receivables_series WHERE id = NEW.series_id) <> 'payment' THEN
    RAISE EXCEPTION 'a payment is numbered by a payment series'
      USING ERRCODE = 'check_violation';
  END IF;

  IF NEW.payment_number IS DISTINCT FROM COALESCE(
    (SELECT max(payment_number) FROM finance_receivables_payments
     WHERE series_id = NEW.series_id), 0) + 1 THEN
    RAISE EXCEPTION 'payment % is not the next number in series %',
      NEW.payment_number, NEW.series_id
      USING ERRCODE = 'check_violation';
  END IF;

  NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_check_allocation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM finance_receivables_documents d
    JOIN finance_receivables_payments p ON p.id = NEW.payment_id
    WHERE d.id = NEW.document_id AND d.kind = 'invoice' AND d.status = 'issued'
      AND d.customer_id = p.customer_id
  ) THEN
    RAISE EXCEPTION 'a payment is allocated to an issued invoice of its customer'
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_setup_change_stamp() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_reversal_stamp() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.reversed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_receivables_check_payment() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
BEGIN
  IF TG_TABLE_NAME = 'finance_receivables_payments' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.payment_id;
  END IF;

  IF (SELECT p.amount - COALESCE(sum(a.amount), 0)
      FROM finance_receivables_payments p
      LEFT JOIN finance_receivables_allocations a ON a.payment_id = p.id
      WHERE p.id = checked_id GROUP BY p.amount) <> 0 THEN
    RAISE EXCEPTION 'payment % allocations do not equal its amount', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_receivables_allocations a
    WHERE a.payment_id = checked_id
      AND finance_receivables_outstanding(a.document_id) < 0
  ) THEN
    RAISE EXCEPTION 'payment % settles an invoice beyond what it owes', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

CREATE TRIGGER finance_receivables_setups_identity BEFORE UPDATE ON finance_receivables_setups
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_identity('tenant_id', 'company_id');
CREATE TRIGGER finance_receivables_setups_kept BEFORE DELETE ON finance_receivables_setups
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_series_identity BEFORE UPDATE ON finance_receivables_series
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_identity('tenant_id', 'company_id', 'code', 'kind', 'prefix', 'number_width');
CREATE TRIGGER finance_receivables_series_kept BEFORE DELETE ON finance_receivables_series
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_tax_codes_identity BEFORE UPDATE ON finance_receivables_tax_codes
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_identity('tenant_id', 'company_id', 'code', 'rate');
CREATE TRIGGER finance_receivables_tax_codes_kept BEFORE DELETE ON finance_receivables_tax_codes
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_customers_identity BEFORE UPDATE ON finance_receivables_customers
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_identity('tenant_id', 'company_id', 'code');
CREATE TRIGGER finance_receivables_customers_kept BEFORE DELETE ON finance_receivables_customers
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();

CREATE TRIGGER finance_receivables_setup_changes_append_only BEFORE UPDATE OR DELETE ON finance_receivables_setup_changes
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_payments_append_only BEFORE UPDATE OR DELETE ON finance_receivables_payments
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_allocations_append_only BEFORE UPDATE OR DELETE ON finance_receivables_allocations
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_payment_reversals_append_only BEFORE UPDATE OR DELETE ON finance_receivables_payment_reversals
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();

CREATE TRIGGER finance_receivables_setups_no_truncate BEFORE TRUNCATE ON finance_receivables_setups
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_setup_changes_no_truncate BEFORE TRUNCATE ON finance_receivables_setup_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_series_no_truncate BEFORE TRUNCATE ON finance_receivables_series
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_tax_codes_no_truncate BEFORE TRUNCATE ON finance_receivables_tax_codes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_customers_no_truncate BEFORE TRUNCATE ON finance_receivables_customers
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_documents_no_truncate BEFORE TRUNCATE ON finance_receivables_documents
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_document_lines_no_truncate BEFORE TRUNCATE ON finance_receivables_document_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_document_tax_lines_no_truncate BEFORE TRUNCATE ON finance_receivables_document_tax_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_payments_no_truncate BEFORE TRUNCATE ON finance_receivables_payments
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_allocations_no_truncate BEFORE TRUNCATE ON finance_receivables_allocations
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_payment_reversals_no_truncate BEFORE TRUNCATE ON finance_receivables_payment_reversals
  FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();

CREATE TRIGGER finance_receivables_document_lines_draft_only BEFORE INSERT OR UPDATE OR DELETE ON finance_receivables_document_lines
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_guard_draft_line();
CREATE TRIGGER finance_receivables_document_tax_lines_draft_only BEFORE INSERT OR UPDATE OR DELETE ON finance_receivables_document_tax_lines
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_guard_draft_line();
CREATE CONSTRAINT TRIGGER finance_receivables_payments_settled AFTER INSERT ON finance_receivables_payments
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_check_payment();
CREATE CONSTRAINT TRIGGER finance_receivables_allocations_settled AFTER INSERT ON finance_receivables_allocations
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_check_payment();

CREATE TRIGGER finance_receivables_documents_kept BEFORE DELETE ON finance_receivables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_documents_guard BEFORE UPDATE ON finance_receivables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_guard_document();
CREATE TRIGGER finance_receivables_documents_draft BEFORE INSERT ON finance_receivables_documents
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_insert_draft();
CREATE TRIGGER finance_receivables_payments_number BEFORE INSERT ON finance_receivables_payments
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_number_payment();
CREATE TRIGGER finance_receivables_allocations_invoice BEFORE INSERT ON finance_receivables_allocations
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_check_allocation();
CREATE TRIGGER finance_receivables_setup_changes_stamp BEFORE INSERT ON finance_receivables_setup_changes
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_setup_change_stamp();
CREATE TRIGGER finance_receivables_payment_reversals_stamp BEFORE INSERT ON finance_receivables_payment_reversals
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_reversal_stamp();
