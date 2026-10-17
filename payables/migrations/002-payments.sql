-- Mirrors Bilimbi Finance.Payables payment and payment-request migrations.

CREATE TABLE finance_payables_payments (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  payment_number bigint NOT NULL,
  payment_date date NOT NULL,
  party_key varchar(100) NOT NULL,
  party_name varchar(200) NOT NULL,
  paid_from_account_id bigint NOT NULL,
  amount numeric(20, 4) NOT NULL,
  currency_code varchar(3) NOT NULL,
  reference varchar(100),
  description varchar(500),
  reverses_payment_id bigint,
  entry_id bigint NOT NULL,
  entry_number bigint NOT NULL,
  posted_by_user_id bigint NOT NULL,
  posted_by_impersonator_id bigint,
  posted_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payables_payments_amount CHECK (amount > 0),
  CONSTRAINT finance_payables_payments_currency_code CHECK (currency_code ~ '^[A-Z]{3}$'),
  CONSTRAINT finance_payables_payments_payment_number CHECK (payment_number > 0),
  CONSTRAINT finance_payables_payments_not_own_reversal
    CHECK (reverses_payment_id IS NULL OR reverses_payment_id <> id)
);

CREATE UNIQUE INDEX finance_payables_payments_id_company_unique
  ON finance_payables_payments (id, company_id);

ALTER TABLE finance_payables_payments
  ADD CONSTRAINT finance_payables_payments_reverses_payment_fkey
  FOREIGN KEY (reverses_payment_id, company_id)
  REFERENCES finance_payables_payments (id, company_id) ON DELETE RESTRICT;

CREATE UNIQUE INDEX finance_payables_payments_company_number_unique
  ON finance_payables_payments (company_id, payment_number);

CREATE UNIQUE INDEX finance_payables_payments_reversed_once
  ON finance_payables_payments (reverses_payment_id)
  WHERE reverses_payment_id IS NOT NULL;

CREATE INDEX finance_payables_payments_company_date_index
  ON finance_payables_payments (tenant_id, company_id, payment_date);

CREATE TABLE finance_payables_payment_allocations (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  payment_id bigint NOT NULL,
  line_number integer NOT NULL,
  document_id bigint NOT NULL,
  amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_payables_payment_allocations_payment_fkey
    FOREIGN KEY (payment_id, company_id)
    REFERENCES finance_payables_payments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_allocations_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_allocations_line_number CHECK (line_number > 0),
  CONSTRAINT finance_payables_payment_allocations_amount CHECK (amount > 0)
);

CREATE UNIQUE INDEX finance_payables_payment_allocations_payment_line_unique
  ON finance_payables_payment_allocations (payment_id, line_number);

CREATE UNIQUE INDEX finance_payables_payment_allocations_payment_document_unique
  ON finance_payables_payment_allocations (payment_id, document_id);

CREATE INDEX finance_payables_payment_allocations_document_index
  ON finance_payables_payment_allocations (company_id, document_id);

CREATE FUNCTION finance_payables_number_payment() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.payment_number := COALESCE(
    (SELECT max(payment_number) FROM finance_payables_payments
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.posted_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_refuse_posted_allocation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_payables_payments
    WHERE id = NEW.payment_id AND NOT (pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress')
  ) THEN
    RAISE EXCEPTION 'payment % is already posted: an allocation cannot be added',
      NEW.payment_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_check_payment() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
  payment record;
  allocated numeric;
BEGIN
  IF TG_TABLE_NAME = 'finance_payables_payments' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.payment_id;
  END IF;

  SELECT amount, reverses_payment_id INTO payment
  FROM finance_payables_payments WHERE id = checked_id;

  SELECT COALESCE(sum(amount), 0) INTO allocated
  FROM finance_payables_payment_allocations WHERE payment_id = checked_id;

  IF payment.reverses_payment_id IS NULL AND allocated <> payment.amount THEN
    RAISE EXCEPTION 'payment % allocations differ from its amount', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF payment.reverses_payment_id IS NOT NULL AND allocated <> 0 THEN
    RAISE EXCEPTION 'payment reversal % takes no allocations', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

CREATE TABLE finance_payables_payment_requests (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  request_number bigint NOT NULL,
  payment_date date NOT NULL,
  party_key varchar(100) NOT NULL,
  party_name varchar(200) NOT NULL,
  paid_from_account_id bigint NOT NULL,
  amount numeric(20, 4) NOT NULL,
  currency_code varchar(3) NOT NULL,
  reference varchar(100),
  description varchar(500),
  submitted_by_user_id bigint NOT NULL,
  submitted_by_impersonator_id bigint,
  submitted_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payables_payment_requests_amount CHECK (amount > 0),
  CONSTRAINT finance_payables_payment_requests_currency_code
    CHECK (currency_code ~ '^[A-Z]{3}$'),
  CONSTRAINT finance_payables_payment_requests_request_number CHECK (request_number > 0)
);

CREATE UNIQUE INDEX finance_payables_payment_requests_id_company_unique
  ON finance_payables_payment_requests (id, company_id);

CREATE UNIQUE INDEX finance_payables_payment_requests_company_number_unique
  ON finance_payables_payment_requests (company_id, request_number);

CREATE INDEX finance_payables_payment_requests_tenant_id_company_id_index
  ON finance_payables_payment_requests (tenant_id, company_id);

CREATE TABLE finance_payables_payment_request_allocations (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  request_id bigint NOT NULL,
  line_number integer NOT NULL,
  document_id bigint NOT NULL,
  amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_payables_payment_request_allocations_request_fkey
    FOREIGN KEY (request_id, company_id)
    REFERENCES finance_payables_payment_requests (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_request_allocations_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_payables_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_request_allocations_line_number
    CHECK (line_number > 0),
  CONSTRAINT finance_payables_payment_request_allocations_amount CHECK (amount > 0)
);

CREATE UNIQUE INDEX finance_payables_payment_request_allocations_line_unique
  ON finance_payables_payment_request_allocations (request_id, line_number);

CREATE UNIQUE INDEX finance_payables_payment_request_allocations_document_unique
  ON finance_payables_payment_request_allocations (request_id, document_id);

CREATE INDEX finance_payables_request_allocations_tenant_company_index
  ON finance_payables_payment_request_allocations (tenant_id, company_id);

CREATE TABLE finance_payables_payment_request_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  request_id bigint NOT NULL,
  event varchar(16) NOT NULL,
  reason varchar(500),
  payment_id bigint,
  actor_user_id bigint NOT NULL,
  actor_impersonator_id bigint,
  occurred_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payables_payment_request_events_request_fkey
    FOREIGN KEY (request_id, company_id)
    REFERENCES finance_payables_payment_requests (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_request_events_payment_fkey
    FOREIGN KEY (payment_id, company_id)
    REFERENCES finance_payables_payments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payables_payment_request_events_event
    CHECK (event IN ('posted', 'rejected')),
  CONSTRAINT finance_payables_payment_request_events_payment
    CHECK ((event = 'posted') = (payment_id IS NOT NULL)),
  CONSTRAINT finance_payables_payment_request_events_reason
    CHECK (event <> 'rejected' OR (reason IS NOT NULL AND btrim(reason) <> ''))
);

CREATE UNIQUE INDEX finance_payables_payment_request_events_request_unique
  ON finance_payables_payment_request_events (request_id);

CREATE UNIQUE INDEX finance_payables_payment_request_events_payment_unique
  ON finance_payables_payment_request_events (payment_id) WHERE payment_id IS NOT NULL;

CREATE INDEX finance_payables_payment_request_events_tenant_company_index
  ON finance_payables_payment_request_events (tenant_id, company_id);

CREATE FUNCTION finance_payables_number_payment_request() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.request_number := COALESCE(
    (SELECT max(request_number) FROM finance_payables_payment_requests
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.submitted_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_refuse_submitted_request_allocation()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_payables_payment_requests
    WHERE id = NEW.request_id AND NOT (pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress')
  ) THEN
    RAISE EXCEPTION 'payment request % is already submitted: an allocation cannot be added',
      NEW.request_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payables_check_payment_request() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
BEGIN
  IF TG_TABLE_NAME = 'finance_payables_payment_requests' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.request_id;
  END IF;

  IF (SELECT amount FROM finance_payables_payment_requests WHERE id = checked_id)
     IS DISTINCT FROM
     (SELECT sum(amount) FROM finance_payables_payment_request_allocations
      WHERE request_id = checked_id) THEN
    RAISE EXCEPTION 'payment request % allocations differ from its amount', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

CREATE FUNCTION finance_payables_payment_request_decided() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.event = 'posted' AND NOT EXISTS (
    SELECT 1
    FROM finance_payables_payment_requests r
    JOIN finance_payables_payments p ON p.id = NEW.payment_id AND p.company_id = r.company_id
    WHERE r.id = NEW.request_id AND r.company_id = NEW.company_id
      AND p.reverses_payment_id IS NULL
      AND p.party_key = r.party_key AND p.payment_date = r.payment_date
      AND p.paid_from_account_id = r.paid_from_account_id AND p.amount = r.amount
  ) THEN
    RAISE EXCEPTION 'payment request % is posted only by a payment of what it requested',
      NEW.request_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE TRIGGER finance_payables_payments_append_only BEFORE UPDATE OR DELETE ON finance_payables_payments FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payments_no_truncate BEFORE TRUNCATE ON finance_payables_payments FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_allocations_append_only BEFORE UPDATE OR DELETE ON finance_payables_payment_allocations FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_allocations_no_truncate BEFORE TRUNCATE ON finance_payables_payment_allocations FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_requests_append_only BEFORE UPDATE OR DELETE ON finance_payables_payment_requests FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_requests_no_truncate BEFORE TRUNCATE ON finance_payables_payment_requests FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_request_allocations_append_only BEFORE UPDATE OR DELETE ON finance_payables_payment_request_allocations FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_request_allocations_no_truncate BEFORE TRUNCATE ON finance_payables_payment_request_allocations FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_request_events_append_only BEFORE UPDATE OR DELETE ON finance_payables_payment_request_events FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payment_request_events_no_truncate BEFORE TRUNCATE ON finance_payables_payment_request_events FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

CREATE TRIGGER finance_payables_payments_insert BEFORE INSERT ON finance_payables_payments FOR EACH ROW EXECUTE FUNCTION finance_payables_number_payment();

CREATE TRIGGER finance_payables_payment_allocations_insert BEFORE INSERT ON finance_payables_payment_allocations FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_posted_allocation();

CREATE TRIGGER finance_payables_payment_requests_insert BEFORE INSERT ON finance_payables_payment_requests FOR EACH ROW EXECUTE FUNCTION finance_payables_number_payment_request();

CREATE TRIGGER finance_payables_payment_request_allocations_insert BEFORE INSERT ON finance_payables_payment_request_allocations FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_submitted_request_allocation();

CREATE TRIGGER finance_payables_payment_request_events_insert BEFORE INSERT ON finance_payables_payment_request_events FOR EACH ROW EXECUTE FUNCTION finance_payables_payment_request_decided();

CREATE CONSTRAINT TRIGGER finance_payables_payments_allocated AFTER INSERT ON finance_payables_payments DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_payment();

CREATE CONSTRAINT TRIGGER finance_payables_payment_allocations_allocated AFTER INSERT ON finance_payables_payment_allocations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_payment();

CREATE CONSTRAINT TRIGGER finance_payables_payment_requests_allocated AFTER INSERT ON finance_payables_payment_requests DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_payment_request();

CREATE CONSTRAINT TRIGGER finance_payables_payment_request_allocations_allocated AFTER INSERT ON finance_payables_payment_request_allocations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_check_payment_request();
