-- finance/einvoice 001: the fiscal documents a company issues through a
-- clearance authority, the signed submissions sent for them, the status
-- events the authority's answers produce, and each company's overrides of the
-- authority's code lists. The end state of the Bilimbi e-invoice schema for
-- these tables (b-dom-finance 4abfaf829321f8540581d833752a086b21b45ba9),
-- including the ledger entry a status event posted or reversed.
--
-- A document, its lines, a submission and a status event are append-only: a
-- trigger refuses UPDATE, DELETE and TRUNCATE, so a cancellation or a
-- rejection is a new status event, never an edit. The database stamps each
-- record's `recorded_at`, numbers a document's status events 1, 2, 3, ...,
-- refuses a line added to a document recorded in an earlier transaction,
-- checks a submission's SHA-256 against its payload bytes, and refuses a
-- commit in which a document has no lines or its totals differ from its
-- lines'. A document's `posted` event names the ledger entry its validation
-- posted and its `posting_reversed` event the entry that reversed it; a
-- partial unique index lets the database refuse a second of either. Rows of
-- one company reference each other through composite (id, company_id)
-- foreign keys.
--
-- The authority's code lists ship with the module as reference data; a
-- company's overrides rename, retire or add a code for that company only. An
-- override is not a fiscal record, so it stays editable.

CREATE TABLE finance_einvoice_documents (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  authority varchar(32) NOT NULL,
  environment varchar(16) NOT NULL,
  document_type varchar(8) NOT NULL,
  code_number varchar(50) NOT NULL,
  issued_at timestamp(0) NOT NULL,
  currency_code varchar(3) NOT NULL,
  exchange_rate numeric(20,6),
  supplier jsonb NOT NULL,
  buyer jsonb NOT NULL,
  details jsonb NOT NULL DEFAULT '{}',
  original_document_id bigint,
  line_extension_amount numeric(20,2) NOT NULL,
  tax_exclusive_amount numeric(20,2) NOT NULL,
  tax_amount numeric(20,2) NOT NULL,
  tax_inclusive_amount numeric(20,2) NOT NULL,
  payable_amount numeric(20,2) NOT NULL,
  issued_by_user_id bigint NOT NULL,
  issued_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_einvoice_documents_environment
    CHECK (environment IN ('sandbox', 'production')),
  CONSTRAINT finance_einvoice_documents_currency_code CHECK (currency_code ~ '^[A-Z]{3}$'),
  CONSTRAINT finance_einvoice_documents_exchange_rate
    CHECK (exchange_rate IS NULL OR exchange_rate > 0),
  CONSTRAINT finance_einvoice_documents_not_own_original
    CHECK (original_document_id IS NULL OR original_document_id <> id),
  -- Every total is non-negative and the totals add up: there are no
  -- document-level allowances, charges or rounding yet.
  CONSTRAINT finance_einvoice_documents_totals CHECK (
    line_extension_amount >= 0 AND tax_amount >= 0
    AND tax_exclusive_amount = line_extension_amount
    AND tax_inclusive_amount = tax_exclusive_amount + tax_amount
    AND payable_amount = tax_inclusive_amount)
);
CREATE UNIQUE INDEX finance_einvoice_documents_id_company_unique
  ON finance_einvoice_documents (id, company_id);
ALTER TABLE finance_einvoice_documents ADD CONSTRAINT finance_einvoice_documents_original_fkey
  FOREIGN KEY (original_document_id, company_id)
  REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT;
-- The authority requires a code number unique to its supplier; sandbox
-- documents are tests and never take a production number.
CREATE UNIQUE INDEX finance_einvoice_documents_code_number_unique
  ON finance_einvoice_documents (company_id, environment, code_number);
CREATE INDEX finance_einvoice_documents_tenant_id_company_id_issued_at_index
  ON finance_einvoice_documents (tenant_id, company_id, issued_at);

CREATE TABLE finance_einvoice_document_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  line_number integer NOT NULL,
  description varchar(300) NOT NULL,
  classification_code varchar(16) NOT NULL,
  tariff_code varchar(32),
  origin_country_code varchar(3),
  quantity numeric(20,6) NOT NULL,
  unit_code varchar(16),
  unit_price numeric(20,6) NOT NULL,
  subtotal numeric(20,2) NOT NULL,
  discount_amount numeric(20,2) NOT NULL DEFAULT 0,
  charge_amount numeric(20,2) NOT NULL DEFAULT 0,
  line_extension_amount numeric(20,2) NOT NULL,
  tax_type varchar(8) NOT NULL,
  tax_rate numeric(9,4),
  tax_amount numeric(20,2) NOT NULL,
  tax_exemption_reason varchar(300),
  CONSTRAINT finance_einvoice_document_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_document_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_einvoice_document_lines_amounts CHECK (
    quantity > 0 AND unit_price >= 0 AND subtotal >= 0
    AND discount_amount >= 0 AND charge_amount >= 0 AND tax_amount >= 0
    AND line_extension_amount >= 0
    AND line_extension_amount = subtotal - discount_amount + charge_amount),
  CONSTRAINT finance_einvoice_document_lines_tax_rate
    CHECK (tax_rate IS NULL OR (tax_rate >= 0 AND tax_rate <= 100))
);
CREATE UNIQUE INDEX finance_einvoice_document_lines_document_line_unique
  ON finance_einvoice_document_lines (document_id, line_number);
CREATE INDEX finance_einvoice_document_lines_company_id_index
  ON finance_einvoice_document_lines (company_id);

CREATE TABLE finance_einvoice_submissions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  environment varchar(16) NOT NULL,
  format varchar(8) NOT NULL,
  document_version varchar(8) NOT NULL,
  payload bytea NOT NULL,
  payload_sha256 varchar(64) NOT NULL,
  actor_type varchar(16) NOT NULL,
  actor_user_id bigint,
  actor_impersonator_id bigint,
  system_principal varchar(100),
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_einvoice_submissions_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_submissions_environment
    CHECK (environment IN ('sandbox', 'production')),
  CONSTRAINT finance_einvoice_submissions_format CHECK (format IN ('JSON', 'XML')),
  -- The stored hash is the hash of the stored bytes, whoever wrote them.
  CONSTRAINT finance_einvoice_submissions_payload_sha256
    CHECK (octet_length(payload) > 0 AND payload_sha256 = encode(sha256(payload), 'hex')),
  -- Who recorded a row: a signed-in user (and any impersonator), or a named
  -- system principal acting for the company.
  CONSTRAINT finance_einvoice_submissions_actor CHECK (
    (actor_type = 'user' AND actor_user_id IS NOT NULL AND system_principal IS NULL)
    OR (actor_type = 'system' AND actor_user_id IS NULL AND actor_impersonator_id IS NULL
        AND system_principal IS NOT NULL))
);
CREATE UNIQUE INDEX finance_einvoice_submissions_id_company_unique
  ON finance_einvoice_submissions (id, company_id);
CREATE INDEX finance_einvoice_submissions_company_id_document_id_index
  ON finance_einvoice_submissions (company_id, document_id);

CREATE TABLE finance_einvoice_status_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  sequence integer NOT NULL,
  submission_id bigint,
  event varchar(32) NOT NULL,
  authority_status varchar(32),
  submission_uid varchar(64),
  document_uuid varchar(64),
  long_id varchar(128),
  reason varchar(300),
  http_status integer,
  response jsonb,
  actor_type varchar(16) NOT NULL,
  actor_user_id bigint,
  actor_impersonator_id bigint,
  system_principal varchar(100),
  ledger_entry_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_einvoice_status_events_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_status_events_submission_fkey
    FOREIGN KEY (submission_id, company_id)
    REFERENCES finance_einvoice_submissions (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_status_events_event CHECK (event ~ '^[a-z][a-z_]*$'),
  CONSTRAINT finance_einvoice_status_events_http_status
    CHECK (http_status IS NULL OR (http_status >= 100 AND http_status <= 599)),
  CONSTRAINT finance_einvoice_status_events_ledger_entry_id
    CHECK (ledger_entry_id IS NULL OR ledger_entry_id > 0),
  CONSTRAINT finance_einvoice_status_events_actor CHECK (
    (actor_type = 'user' AND actor_user_id IS NOT NULL AND system_principal IS NULL)
    OR (actor_type = 'system' AND actor_user_id IS NULL AND actor_impersonator_id IS NULL
        AND system_principal IS NOT NULL))
);
CREATE UNIQUE INDEX finance_einvoice_status_events_document_sequence_unique
  ON finance_einvoice_status_events (document_id, sequence);
CREATE INDEX finance_einvoice_status_events_company_id_document_uuid_index
  ON finance_einvoice_status_events (company_id, document_uuid) WHERE document_uuid IS NOT NULL;
-- Finds, per company, the documents whose postings the reconciliation page
-- compares.
CREATE INDEX finance_einvoice_status_events_ledger_entry_index
  ON finance_einvoice_status_events (company_id, document_id, event)
  WHERE ledger_entry_id IS NOT NULL;
-- One ledger entry and one reversal per document: no race between two
-- callers can leave a document with two entries or two reversals.
CREATE UNIQUE INDEX finance_einvoice_status_events_posted_unique
  ON finance_einvoice_status_events (document_id) WHERE event = 'posted';
CREATE UNIQUE INDEX finance_einvoice_status_events_reversed_unique
  ON finance_einvoice_status_events (document_id) WHERE event = 'posting_reversed';

CREATE TABLE finance_einvoice_code_overrides (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  authority varchar(32) NOT NULL,
  list varchar(32) NOT NULL,
  code varchar(16) NOT NULL,
  description varchar(500),
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL
);
CREATE UNIQUE INDEX finance_einvoice_code_overrides_unique
  ON finance_einvoice_code_overrides (company_id, authority, list, code);
CREATE INDEX finance_einvoice_code_overrides_tenant_id_company_id_index
  ON finance_einvoice_code_overrides (tenant_id, company_id);

CREATE FUNCTION finance_einvoice_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance e-invoicing keeps % rows: % is refused; record a new status event',
    TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE TRIGGER finance_einvoice_documents_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_documents
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_documents_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_documents
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_document_lines_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_document_lines
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_document_lines_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_document_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_submissions_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_submissions
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_submissions_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_submissions
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_status_events_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_status_events
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_status_events_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_status_events
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();

-- The database's clock, whatever the inserting writer sent.
CREATE FUNCTION finance_einvoice_stamp_recorded_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_einvoice_documents_recorded_at
  BEFORE INSERT ON finance_einvoice_documents
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_stamp_recorded_at();
CREATE TRIGGER finance_einvoice_submissions_recorded_at
  BEFORE INSERT ON finance_einvoice_submissions
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_stamp_recorded_at();
CREATE TRIGGER finance_einvoice_status_events_recorded_at
  BEFORE INSERT ON finance_einvoice_status_events
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_stamp_recorded_at();

-- A document's events are numbered 1, 2, 3, ... in recording order; with no
-- deletion a number is never skipped or reused, and a concurrent writer that
-- takes the same number fails on the unique index.
CREATE FUNCTION finance_einvoice_number_status_event() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.sequence := COALESCE(
    (SELECT max(sequence) FROM finance_einvoice_status_events
     WHERE document_id = NEW.document_id), 0) + 1;
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_einvoice_status_events_number
  BEFORE INSERT ON finance_einvoice_status_events
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_number_status_event();

-- A document is recorded once its inserting transaction is no longer in
-- progress. pg_xact_status is NULL for a transaction older than the retained
-- commit log, so IS DISTINCT FROM keeps old documents refused.
CREATE FUNCTION finance_einvoice_refuse_recorded_line() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_einvoice_documents
    WHERE id = NEW.document_id
      AND pg_xact_status(
            ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
          ) IS DISTINCT FROM 'in progress'
  ) THEN
    RAISE EXCEPTION 'document % is already recorded: a line cannot be added; issue a note',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;
CREATE TRIGGER finance_einvoice_document_lines_recorded_document
  BEFORE INSERT ON finance_einvoice_document_lines
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_recorded_line();

-- At commit a document has lines, and its totals are its lines' totals.
CREATE FUNCTION finance_einvoice_check_totals() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
  document finance_einvoice_documents%ROWTYPE;
BEGIN
  IF TG_TABLE_NAME = 'finance_einvoice_documents' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.document_id;
  END IF;

  SELECT * INTO document FROM finance_einvoice_documents WHERE id = checked_id;

  IF NOT EXISTS (
    SELECT 1 FROM finance_einvoice_document_lines WHERE document_id = checked_id
  ) THEN
    RAISE EXCEPTION 'document % has no lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF (SELECT sum(line_extension_amount) FROM finance_einvoice_document_lines
      WHERE document_id = checked_id) <> document.line_extension_amount
    OR (SELECT sum(tax_amount) FROM finance_einvoice_document_lines
        WHERE document_id = checked_id) <> document.tax_amount THEN
    RAISE EXCEPTION 'document % totals differ from its lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;
CREATE CONSTRAINT TRIGGER finance_einvoice_documents_match_lines
  AFTER INSERT ON finance_einvoice_documents
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_check_totals();
CREATE CONSTRAINT TRIGGER finance_einvoice_document_lines_match_lines
  AFTER INSERT ON finance_einvoice_document_lines
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_check_totals();
