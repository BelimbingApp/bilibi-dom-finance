-- Mirrors Bilimbi Finance.Payables matching (20261014100000) at its end state:
-- the held status, match events, lines and exceptions, and the transition rule
-- that a bill naming order lines posts only with its match record. Bilimbi's
-- runtime OrderSource is replaced by Payables' own copy of the order lines and
-- goods receipts it matches against, kept from Supply Chain's domain events;
-- nothing here references another container's tables.

ALTER TABLE finance_payables_document_lines
  ADD CONSTRAINT finance_payables_document_lines_order_line_id
    CHECK (order_line_id IS NULL OR order_line_id > 0);

CREATE INDEX finance_payables_document_lines_order_line_index
  ON finance_payables_document_lines (company_id, order_line_id)
  WHERE order_line_id IS NOT NULL;

ALTER TABLE finance_payables_documents DROP CONSTRAINT finance_payables_documents_status;

ALTER TABLE finance_payables_documents ADD CONSTRAINT finance_payables_documents_status
  CHECK (status IN ('draft', 'approved', 'held', 'posted', 'voided'));

ALTER TABLE finance_payables_documents DROP CONSTRAINT finance_payables_documents_approval;

ALTER TABLE finance_payables_documents ADD CONSTRAINT finance_payables_documents_approval
  CHECK ((approved_at IS NULL) = (approved_by_user_id IS NULL)
    AND (status NOT IN ('approved', 'held', 'posted') OR approved_at IS NOT NULL)
    AND (status <> 'draft' OR approved_at IS NULL));

CREATE TABLE finance_payables_match_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  event varchar(20) NOT NULL,
  reason varchar(500),
  held_event_id bigint,
  actor_user_id bigint NOT NULL,
  actor_impersonator_id bigint,
  occurred_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payables_match_events_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_match_events_event
    CHECK (event IN ('held', 'exception_approved', 'posted')),
  CONSTRAINT finance_payables_match_events_reason
    CHECK (event <> 'exception_approved' OR reason IS NOT NULL),
  CONSTRAINT finance_payables_match_events_held_event
    CHECK ((held_event_id IS NULL) = (event <> 'exception_approved'))
);

CREATE UNIQUE INDEX finance_payables_match_events_id_company_unique
  ON finance_payables_match_events (id, company_id);

ALTER TABLE finance_payables_match_events
  ADD CONSTRAINT finance_payables_match_events_held_event_fkey
    FOREIGN KEY (held_event_id, company_id)
    REFERENCES finance_payables_match_events (id, company_id) ON DELETE RESTRICT;

CREATE INDEX finance_payables_match_events_document_index
  ON finance_payables_match_events (company_id, document_id);

CREATE INDEX finance_payables_match_events_tenant_id_company_id_index
  ON finance_payables_match_events (tenant_id, company_id);

-- The figures one posting attempt compared for one bill line: what the copy
-- of the order source said then, what this module had billed before, and the
-- tolerances in force. Ordered and received figures are absent when the
-- source could not answer.
CREATE TABLE finance_payables_match_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  event_id bigint NOT NULL,
  document_id bigint NOT NULL,
  line_number integer NOT NULL,
  order_line_id bigint NOT NULL,
  order_number bigint,
  billed_quantity numeric(20, 4) NOT NULL,
  billed_unit_price numeric(20, 4) NOT NULL,
  previously_billed_quantity numeric(20, 4) NOT NULL,
  ordered_quantity numeric(20, 4),
  ordered_unit_price numeric(20, 4),
  received_quantity numeric(20, 4),
  quantity_tolerance_kind varchar(16),
  quantity_tolerance numeric(20, 4),
  price_tolerance_kind varchar(16),
  price_tolerance numeric(20, 4),
  CONSTRAINT finance_payables_match_lines_event_fkey
    FOREIGN KEY (event_id, company_id)
    REFERENCES finance_payables_match_events (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_match_lines_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_match_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_payables_match_lines_order_line_id CHECK (order_line_id > 0),
  CONSTRAINT finance_payables_match_lines_quantities
    CHECK (billed_quantity > 0 AND billed_unit_price >= 0
      AND previously_billed_quantity >= 0
      AND (ordered_quantity IS NULL OR ordered_quantity > 0)
      AND (ordered_unit_price IS NULL OR ordered_unit_price >= 0)
      AND (received_quantity IS NULL OR received_quantity >= 0)),
  CONSTRAINT finance_payables_match_lines_tolerances
    CHECK ((quantity_tolerance_kind IS NULL) = (quantity_tolerance IS NULL)
      AND (price_tolerance_kind IS NULL) = (price_tolerance IS NULL))
);

CREATE UNIQUE INDEX finance_payables_match_lines_id_company_unique
  ON finance_payables_match_lines (id, company_id);

CREATE UNIQUE INDEX finance_payables_match_lines_event_line_unique
  ON finance_payables_match_lines (event_id, line_number);

CREATE INDEX finance_payables_match_lines_tenant_id_company_id_index
  ON finance_payables_match_lines (tenant_id, company_id);

CREATE TABLE finance_payables_match_exceptions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  match_line_id bigint NOT NULL,
  code varchar(40) NOT NULL,
  detail varchar(500) NOT NULL,
  CONSTRAINT finance_payables_match_exceptions_match_line_fkey
    FOREIGN KEY (match_line_id, company_id)
    REFERENCES finance_payables_match_lines (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_match_exceptions_code
    CHECK (code IN ('order_source_unavailable', 'order_line_not_found',
      'order_line_superseded', 'order_not_open', 'currency_mismatch',
      'quantity_exceeds_ordered', 'quantity_exceeds_received', 'price_differs'))
);

CREATE UNIQUE INDEX finance_payables_match_exceptions_line_code_unique
  ON finance_payables_match_exceptions (match_line_id, code);

CREATE INDEX finance_payables_match_exceptions_tenant_id_company_id_index
  ON finance_payables_match_exceptions (tenant_id, company_id);

-- Payables' copy of an order line of the order source, one per company and
-- source line ID. Goods receipts create it with the facts their snapshot
-- carries; the order's own events (unit price, currency, status, amendment)
-- complete it. A line is matched only once its order facts are present.
CREATE TABLE finance_payables_order_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  order_line_id bigint NOT NULL,
  order_id bigint NOT NULL,
  order_number bigint,
  line_number integer,
  description varchar(500),
  unit varchar(20),
  quantity numeric(20, 4),
  unit_price numeric(20, 4),
  currency_code varchar(3),
  order_status varchar(20),
  superseded_by_amendment_id bigint,
  order_event_key varchar(200),
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_payables_order_lines_ids
    CHECK (order_line_id > 0 AND order_id > 0
      AND (order_number IS NULL OR order_number > 0)
      AND (line_number IS NULL OR line_number > 0)),
  CONSTRAINT finance_payables_order_lines_figures
    CHECK ((quantity IS NULL OR quantity > 0) AND (unit_price IS NULL OR unit_price >= 0)
      AND (currency_code IS NULL OR currency_code ~ '^[A-Z]{3}$')),
  CONSTRAINT finance_payables_order_lines_order_facts
    CHECK ((order_event_key IS NULL)
      OR (quantity IS NOT NULL AND unit_price IS NOT NULL
          AND currency_code IS NOT NULL AND order_status IS NOT NULL))
);

CREATE UNIQUE INDEX finance_payables_order_lines_company_line_unique
  ON finance_payables_order_lines (company_id, order_line_id);

CREATE UNIQUE INDEX finance_payables_order_lines_line_company_unique
  ON finance_payables_order_lines (order_line_id, company_id);

CREATE INDEX finance_payables_order_lines_tenant_id_company_id_index
  ON finance_payables_order_lines (tenant_id, company_id);

-- One goods receipt line or its reversal, as the source posted it, once per
-- event key. The quantity received against an order line is the receipts'
-- accepted quantities less the reversals'.
CREATE TABLE finance_payables_order_receipts (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  order_line_id bigint NOT NULL,
  event_key varchar(200) NOT NULL,
  kind varchar(20) NOT NULL,
  posting_id bigint NOT NULL,
  receipt_id bigint NOT NULL,
  receipt_number bigint NOT NULL,
  posted_at timestamp(6) NOT NULL,
  received_quantity numeric(20, 4) NOT NULL,
  rejected_quantity numeric(20, 4) NOT NULL,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_payables_order_receipts_order_line_fkey
    FOREIGN KEY (order_line_id, company_id)
    REFERENCES finance_payables_order_lines (order_line_id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_order_receipts_kind CHECK (kind IN ('receipt', 'reversal')),
  CONSTRAINT finance_payables_order_receipts_quantities
    CHECK (received_quantity >= 0 AND rejected_quantity >= 0)
);

CREATE UNIQUE INDEX finance_payables_order_receipts_event_unique
  ON finance_payables_order_receipts (tenant_id, event_key);

CREATE INDEX finance_payables_order_receipts_order_line_index
  ON finance_payables_order_receipts (company_id, order_line_id);

-- A match event is recorded for a bill in the status its kind expects, and
-- the database stamps its time. An approval needs a hold to approve, names
-- that hold, and approves it once; a posting of a held bill needs that
-- approval unless it found no exception (the transition trigger reads the
-- exceptions, which land after the event).
CREATE FUNCTION finance_payables_match_event_recorded() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  document record;
  latest text;
  latest_id bigint;
BEGIN
  SELECT document_type, status INTO document
  FROM finance_payables_documents WHERE id = NEW.document_id;

  SELECT event, id INTO latest, latest_id FROM finance_payables_match_events
  WHERE document_id = NEW.document_id ORDER BY id DESC LIMIT 1;

  IF document.document_type <> 'bill' THEN
    RAISE EXCEPTION 'payables document % is not a bill: only a bill is matched',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.event IN ('held', 'posted') AND document.status NOT IN ('approved', 'held') THEN
    RAISE EXCEPTION 'payables document % is %: only an approved or held bill is matched',
      NEW.document_id, document.status
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.event = 'exception_approved'
     AND (document.status <> 'held' OR latest IS DISTINCT FROM 'held') THEN
    RAISE EXCEPTION 'payables document % has no held match exception to approve',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.event = 'exception_approved' AND NEW.held_event_id IS DISTINCT FROM latest_id THEN
    RAISE EXCEPTION 'payables document % approval must name its latest hold',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

-- Match lines are inserted only with their event, and exceptions only with
-- their line, in the transaction that records them.
CREATE FUNCTION finance_payables_refuse_closed_match() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  closed boolean;
BEGIN
  IF TG_TABLE_NAME = 'finance_payables_match_lines' THEN
    SELECT EXISTS (
      SELECT 1 FROM finance_payables_match_events
      WHERE id = NEW.event_id
        AND NOT (pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress')
    ) INTO closed;
  ELSE
    SELECT EXISTS (
      SELECT 1 FROM finance_payables_match_lines
      WHERE id = NEW.match_line_id
        AND NOT (pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress')
    ) INTO closed;
  END IF;

  IF closed THEN
    RAISE EXCEPTION 'a payables match record is complete when recorded: % refused on %',
      TG_OP, TG_TABLE_NAME
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- The first migration's transition rule with the held status: approved ->
-- held, held -> posted or voided; a held bill posts only after an approval
-- later than its last hold unless the posting match found no exception; and
-- a bill naming order lines posts only with its posted match record, written
-- in the same transaction.
CREATE OR REPLACE FUNCTION finance_payables_document_transition() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  stamp timestamp := clock_timestamp() AT TIME ZONE 'UTC';
  last_hold bigint;
  posting bigint;
BEGIN
  IF NOT ((OLD.status = 'draft' AND NEW.status IN ('approved', 'voided'))
          OR (OLD.status = 'approved' AND NEW.status IN ('posted', 'held', 'voided'))
          OR (OLD.status = 'held' AND NEW.status IN ('posted', 'voided'))) THEN
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
    IF OLD.document_type = 'bill' AND EXISTS (
      SELECT 1 FROM finance_payables_document_lines
      WHERE document_id = OLD.id AND order_line_id IS NOT NULL
    ) THEN
      SELECT id INTO posting FROM finance_payables_match_events
      WHERE document_id = OLD.id AND event = 'posted'
        AND pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress'
      ORDER BY id DESC LIMIT 1;

      IF posting IS NULL THEN
        RAISE EXCEPTION 'payables document % names order lines: it posts with its match record',
          OLD.id
          USING ERRCODE = 'restrict_violation';
      END IF;

      IF OLD.status = 'held' THEN
        SELECT max(id) INTO last_hold FROM finance_payables_match_events
        WHERE document_id = OLD.id AND event = 'held';

        IF NOT EXISTS (
          SELECT 1 FROM finance_payables_match_events
          WHERE document_id = OLD.id AND event = 'exception_approved' AND id > last_hold
        ) AND EXISTS (
          SELECT 1 FROM finance_payables_match_exceptions x
          JOIN finance_payables_match_lines l ON l.id = x.match_line_id
          WHERE l.event_id = posting
        ) THEN
          RAISE EXCEPTION 'payables document % is held: its match exceptions are not approved',
            OLD.id
            USING ERRCODE = 'restrict_violation';
        END IF;
      END IF;
    END IF;

    NEW.posted_at := stamp;
    NEW.document_number := COALESCE(
      (SELECT max(document_number) FROM finance_payables_documents
       WHERE company_id = NEW.company_id AND document_type = NEW.document_type), 0) + 1;
  ELSIF NEW.status = 'voided' THEN
    NEW.voided_at := stamp;
  END IF;

  RETURN NEW;
END
$$;

CREATE TRIGGER finance_payables_match_events_recorded
  BEFORE INSERT ON finance_payables_match_events
  FOR EACH ROW EXECUTE FUNCTION finance_payables_match_event_recorded();

CREATE TRIGGER finance_payables_match_lines_open_event BEFORE INSERT ON finance_payables_match_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_closed_match();

CREATE TRIGGER finance_payables_match_exceptions_open_event BEFORE INSERT ON finance_payables_match_exceptions FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_closed_match();

CREATE TRIGGER finance_payables_match_events_append_only BEFORE UPDATE OR DELETE ON finance_payables_match_events FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_match_events_no_truncate BEFORE TRUNCATE ON finance_payables_match_events FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_match_lines_append_only BEFORE UPDATE OR DELETE ON finance_payables_match_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_match_lines_no_truncate BEFORE TRUNCATE ON finance_payables_match_lines FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_match_exceptions_append_only BEFORE UPDATE OR DELETE ON finance_payables_match_exceptions FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_match_exceptions_no_truncate BEFORE TRUNCATE ON finance_payables_match_exceptions FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_order_lines_kept BEFORE DELETE ON finance_payables_order_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_order_lines_no_truncate BEFORE TRUNCATE ON finance_payables_order_lines FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_order_lines_identity BEFORE UPDATE ON finance_payables_order_lines FOR EACH ROW EXECUTE FUNCTION finance_payables_keep_identity();

CREATE TRIGGER finance_payables_order_receipts_append_only BEFORE UPDATE OR DELETE ON finance_payables_order_receipts FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_order_receipts_no_truncate BEFORE TRUNCATE ON finance_payables_order_receipts FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();
