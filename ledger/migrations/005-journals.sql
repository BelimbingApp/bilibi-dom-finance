-- finance/ledger 005: manual journals, the remaining portion of Bilimbi
-- Finance.Ledger.Migrations.AddDutySeparation (b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9); its duty separation changes,
-- approval exceptions and their reviews are already in 004.
--
-- A manual journal is submitted with its lines, numbered and stamped by the
-- database, and is decided once: approved, naming the entry it posted, or
-- rejected with a reason. A line is added only in the transaction that
-- submits its journal, a deferred constraint trigger refuses a commit in
-- which a journal has no lines or does not balance, and an approval by its
-- submitter (or whoever impersonated them, or anyone they impersonated) is
-- refused unless the same transaction recorded the same-person approval
-- exception for it, which 004's trigger allows only while the company's
-- latest duty separation change turns separation off. Every table here is
-- append-only.

CREATE TABLE finance_ledger_journals (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  journal_number bigint NOT NULL,
  entry_date date NOT NULL,
  description varchar(500) NOT NULL,
  reference varchar(100),
  currency_code varchar(3) NOT NULL,
  submitted_by_user_id bigint NOT NULL,
  submitted_by_impersonator_id bigint,
  submitted_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_journals_book_fkey FOREIGN KEY (company_id, currency_code)
    REFERENCES finance_ledger_books (company_id, currency_code) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journals_journal_number CHECK (journal_number > 0)
);
CREATE UNIQUE INDEX finance_ledger_journals_id_company_unique ON finance_ledger_journals (id, company_id);
CREATE UNIQUE INDEX finance_ledger_journals_company_number_unique ON finance_ledger_journals (company_id, journal_number);
CREATE INDEX finance_ledger_journals_tenant_id_company_id_index ON finance_ledger_journals (tenant_id, company_id);

CREATE TABLE finance_ledger_journal_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  journal_id bigint NOT NULL,
  line_number integer NOT NULL,
  account_id bigint NOT NULL,
  debit numeric(20,4) NOT NULL,
  credit numeric(20,4) NOT NULL,
  description varchar(500),
  CONSTRAINT finance_ledger_journal_lines_journal_fkey FOREIGN KEY (journal_id, company_id)
    REFERENCES finance_ledger_journals (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_lines_account_fkey FOREIGN KEY (account_id, company_id)
    REFERENCES finance_ledger_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_ledger_journal_lines_one_side
    CHECK (debit >= 0 AND credit >= 0 AND (debit = 0) <> (credit = 0))
);
CREATE UNIQUE INDEX finance_ledger_journal_lines_journal_line_unique ON finance_ledger_journal_lines (journal_id, line_number);
CREATE INDEX finance_ledger_journal_lines_tenant_id_company_id_index ON finance_ledger_journal_lines (tenant_id, company_id);

CREATE TABLE finance_ledger_journal_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  journal_id bigint NOT NULL,
  event varchar(16) NOT NULL,
  reason varchar(500),
  entry_id bigint,
  actor_user_id bigint NOT NULL,
  actor_impersonator_id bigint,
  occurred_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_journal_events_journal_fkey FOREIGN KEY (journal_id, company_id)
    REFERENCES finance_ledger_journals (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_events_entry_fkey FOREIGN KEY (entry_id, company_id)
    REFERENCES finance_ledger_entries (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_events_event CHECK (event IN ('approved', 'rejected')),
  CONSTRAINT finance_ledger_journal_events_entry CHECK ((event = 'approved') = (entry_id IS NOT NULL)),
  CONSTRAINT finance_ledger_journal_events_reason
    CHECK (event <> 'rejected' OR (reason IS NOT NULL AND btrim(reason) <> ''))
);
-- A journal is decided once, and an entry posts one journal.
CREATE UNIQUE INDEX finance_ledger_journal_events_journal_unique ON finance_ledger_journal_events (journal_id);
CREATE UNIQUE INDEX finance_ledger_journal_events_entry_unique ON finance_ledger_journal_events (entry_id) WHERE entry_id IS NOT NULL;
CREATE INDEX finance_ledger_journal_events_tenant_id_company_id_index ON finance_ledger_journal_events (tenant_id, company_id);

CREATE FUNCTION finance_ledger_record_journal() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.journal_number := COALESCE((SELECT max(journal_number) FROM finance_ledger_journals
    WHERE company_id = NEW.company_id), 0) + 1;
  NEW.submitted_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

-- Whether the transaction that wrote the journal row is still in progress:
-- see 002's posted-line guard.
CREATE FUNCTION finance_ledger_refuse_submitted_journal_line() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_ledger_journals
    WHERE id = NEW.journal_id
      AND pg_xact_status(
            ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
          ) IS DISTINCT FROM 'in progress'
  ) THEN
    RAISE EXCEPTION 'journal % is already submitted: a line cannot be added', NEW.journal_id
      USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_ledger_check_journal_balance() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
BEGIN
  IF TG_TABLE_NAME = 'finance_ledger_journals' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.journal_id;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM finance_ledger_journal_lines WHERE journal_id = checked_id) THEN
    RAISE EXCEPTION 'journal % has no lines', checked_id USING ERRCODE = 'check_violation';
  END IF;
  IF (SELECT sum(debit) - sum(credit) FROM finance_ledger_journal_lines WHERE journal_id = checked_id) <> 0 THEN
    RAISE EXCEPTION 'journal % does not balance', checked_id USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END
$$;

-- Its submitter approves a journal only with the same-person exception
-- recorded in the same transaction, which the exception's own trigger allows
-- only while separation is off.
CREATE FUNCTION finance_ledger_record_journal_event() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  journal finance_ledger_journals%ROWTYPE;
BEGIN
  SELECT * INTO journal FROM finance_ledger_journals
    WHERE id = NEW.journal_id AND company_id = NEW.company_id;
  IF NEW.event = 'approved' AND FOUND
    AND (COALESCE(NEW.actor_user_id IN (journal.submitted_by_user_id, journal.submitted_by_impersonator_id), false)
      OR COALESCE(NEW.actor_impersonator_id IN (journal.submitted_by_user_id, journal.submitted_by_impersonator_id), false))
    AND NOT EXISTS (
      SELECT 1 FROM finance_ledger_approval_exceptions
      WHERE company_id = NEW.company_id AND source = 'finance/ledger'
        AND subject_type = 'journal' AND subject_id = NEW.journal_id AND same_person
        AND pg_xact_status(
              ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
            ) = 'in progress'
    ) THEN
    RAISE EXCEPTION 'journal % is approved by someone other than whoever submitted it', NEW.journal_id
      USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE TRIGGER finance_ledger_journals_append_only BEFORE UPDATE OR DELETE ON finance_ledger_journals FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journals_no_truncate BEFORE TRUNCATE ON finance_ledger_journals FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journal_lines_append_only BEFORE UPDATE OR DELETE ON finance_ledger_journal_lines FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journal_lines_no_truncate BEFORE TRUNCATE ON finance_ledger_journal_lines FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journal_events_append_only BEFORE UPDATE OR DELETE ON finance_ledger_journal_events FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journal_events_no_truncate BEFORE TRUNCATE ON finance_ledger_journal_events FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journals_record BEFORE INSERT ON finance_ledger_journals FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_journal();
CREATE TRIGGER finance_ledger_journal_events_record BEFORE INSERT ON finance_ledger_journal_events FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_journal_event();
CREATE TRIGGER finance_ledger_journal_lines_submitted_journal BEFORE INSERT ON finance_ledger_journal_lines FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_submitted_journal_line();
CREATE CONSTRAINT TRIGGER finance_ledger_journals_balanced AFTER INSERT ON finance_ledger_journals DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_journal_balance();
CREATE CONSTRAINT TRIGGER finance_ledger_journal_lines_balanced AFTER INSERT ON finance_ledger_journal_lines DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_journal_balance();
