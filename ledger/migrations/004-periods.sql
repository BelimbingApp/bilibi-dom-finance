-- Mirrors Bilimbi Finance.Ledger.Migrations.CreateClosing,
-- AddControlledBookkeeping, AddLateRecordingReviews and the approval evidence
-- portion of AddDutySeparation. Fresh schema only; no legacy adoption.
-- Bookkeeping changes and retained earnings already exist in 001.
-- Duty separation evidence is included because overrides use its exception
-- contract; manual journals and the generic duties facade are a later slice.

CREATE TABLE finance_ledger_fiscal_years (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  start_date date NOT NULL,
  end_date date NOT NULL,
  opened_by_user_id bigint NOT NULL,
  opened_by_impersonator_id bigint,
  opened_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_fiscal_years_book_fkey FOREIGN KEY (company_id)
    REFERENCES finance_ledger_books (company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_fiscal_years_months CHECK (
    extract(day FROM start_date) = 1 AND extract(day FROM end_date + 1) = 1
    AND end_date > start_date AND end_date < start_date + interval '2 years')
);
CREATE UNIQUE INDEX finance_ledger_fiscal_years_id_company_unique ON finance_ledger_fiscal_years (id, company_id);
CREATE UNIQUE INDEX finance_ledger_fiscal_years_company_start_unique ON finance_ledger_fiscal_years (company_id, start_date);
CREATE INDEX finance_ledger_fiscal_years_tenant_id_company_id_index ON finance_ledger_fiscal_years (tenant_id, company_id);

CREATE TABLE finance_ledger_periods (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  fiscal_year_id bigint NOT NULL,
  period_number smallint NOT NULL,
  start_date date NOT NULL,
  end_date date NOT NULL,
  CONSTRAINT finance_ledger_periods_fiscal_year_fkey FOREIGN KEY (fiscal_year_id, company_id)
    REFERENCES finance_ledger_fiscal_years (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_periods_period_number CHECK (period_number > 0),
  CONSTRAINT finance_ledger_periods_one_month CHECK (extract(day FROM start_date) = 1
    AND end_date = (start_date + interval '1 month' - interval '1 day')::date)
);
CREATE UNIQUE INDEX finance_ledger_periods_id_company_unique ON finance_ledger_periods (id, company_id);
CREATE UNIQUE INDEX finance_ledger_periods_id_fiscal_year_unique ON finance_ledger_periods (id, fiscal_year_id);
CREATE UNIQUE INDEX finance_ledger_periods_fiscal_year_number_unique ON finance_ledger_periods (fiscal_year_id, period_number);
CREATE UNIQUE INDEX finance_ledger_periods_company_start_unique ON finance_ledger_periods (company_id, start_date);
CREATE INDEX finance_ledger_periods_tenant_id_company_id_index ON finance_ledger_periods (tenant_id, company_id);

CREATE TABLE finance_ledger_closing_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  event_number bigint NOT NULL,
  fiscal_year_id bigint NOT NULL,
  period_id bigint,
  action varchar(16) NOT NULL,
  reason varchar(500),
  entry_id bigint,
  retained_earnings_account_id bigint,
  recorded_by_user_id bigint NOT NULL,
  recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_closing_events_fiscal_year_fkey FOREIGN KEY (fiscal_year_id, company_id)
    REFERENCES finance_ledger_fiscal_years (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_closing_events_period_fkey FOREIGN KEY (period_id, fiscal_year_id)
    REFERENCES finance_ledger_periods (id, fiscal_year_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_closing_events_entry_fkey FOREIGN KEY (entry_id, company_id)
    REFERENCES finance_ledger_entries (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_closing_events_retained_earnings_fkey FOREIGN KEY (retained_earnings_account_id, company_id)
    REFERENCES finance_ledger_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_closing_events_action CHECK (action IN ('close', 'reopen', 'lock')),
  CONSTRAINT finance_ledger_closing_events_event_number CHECK (event_number > 0),
  CONSTRAINT finance_ledger_closing_events_reopen_reason CHECK (action <> 'reopen' OR (reason IS NOT NULL AND btrim(reason) <> '')),
  CONSTRAINT finance_ledger_closing_events_year_fields CHECK (
    (period_id IS NOT NULL OR action <> 'lock') AND
    ((period_id IS NULL AND action = 'close') OR (entry_id IS NULL AND retained_earnings_account_id IS NULL)))
);
CREATE UNIQUE INDEX finance_ledger_closing_events_company_number_unique ON finance_ledger_closing_events (company_id, event_number);
CREATE UNIQUE INDEX finance_ledger_closing_events_entry_unique ON finance_ledger_closing_events (entry_id) WHERE entry_id IS NOT NULL;
CREATE INDEX finance_ledger_closing_events_fiscal_year_id_event_number_index ON finance_ledger_closing_events (fiscal_year_id, event_number);
CREATE INDEX finance_ledger_closing_events_period_id_event_number_index ON finance_ledger_closing_events (period_id, event_number);
CREATE INDEX finance_ledger_closing_events_tenant_id_company_id_index ON finance_ledger_closing_events (tenant_id, company_id);

CREATE FUNCTION finance_ledger_refuse_closing_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'the finance ledger keeps % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE FUNCTION finance_ledger_period_status(target bigint) RETURNS text
LANGUAGE sql AS $$
  SELECT COALESCE((SELECT CASE action WHEN 'close' THEN 'closed' WHEN 'lock' THEN 'locked' ELSE 'open' END
    FROM finance_ledger_closing_events WHERE period_id = target ORDER BY event_number DESC LIMIT 1), 'open')
$$;
CREATE FUNCTION finance_ledger_fiscal_year_status(target bigint) RETURNS text
LANGUAGE sql AS $$
  SELECT COALESCE((SELECT CASE action WHEN 'close' THEN 'closed' ELSE 'open' END
    FROM finance_ledger_closing_events WHERE fiscal_year_id = target AND period_id IS NULL
    ORDER BY event_number DESC LIMIT 1), 'open')
$$;

CREATE FUNCTION finance_ledger_open_fiscal_year() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  last_end date;
BEGIN
  PERFORM 1 FROM finance_ledger_books WHERE company_id = NEW.company_id FOR NO KEY UPDATE;
  SELECT max(end_date) INTO last_end FROM finance_ledger_fiscal_years WHERE company_id = NEW.company_id;
  IF last_end IS NOT NULL AND NEW.start_date <> last_end + 1 THEN
    RAISE EXCEPTION 'a fiscal year starts on %, the day after the previous one ends', last_end + 1
      USING ERRCODE = 'check_violation';
  END IF;
  NEW.opened_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE FUNCTION finance_ledger_check_fiscal_year_periods() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF (SELECT max(end_date) FROM finance_ledger_periods WHERE fiscal_year_id = NEW.id) IS DISTINCT FROM NEW.end_date THEN
    RAISE EXCEPTION 'fiscal year % is not divided into periods through its end', NEW.id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END
$$;
CREATE FUNCTION finance_ledger_place_period() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  year_start date;
  year_end date;
  year_xmin xid;
  last_number integer;
  last_end date;
BEGIN
  SELECT start_date, end_date, xmin INTO year_start, year_end, year_xmin
  FROM finance_ledger_fiscal_years WHERE id = NEW.fiscal_year_id AND company_id = NEW.company_id;
  IF NOT FOUND THEN RETURN NEW; END IF;
  IF pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | year_xmin::text::bigint)::text::xid8
  ) IS DISTINCT FROM 'in progress' THEN
    RAISE EXCEPTION 'fiscal year % is already open: a period cannot be added', NEW.fiscal_year_id
      USING ERRCODE = 'restrict_violation';
  END IF;
  SELECT max(period_number), max(end_date) INTO last_number, last_end
    FROM finance_ledger_periods WHERE fiscal_year_id = NEW.fiscal_year_id;
  IF NEW.period_number <> COALESCE(last_number, 0) + 1
    OR NEW.start_date <> COALESCE(last_end, year_start - 1) + 1 OR NEW.end_date > year_end THEN
    RAISE EXCEPTION 'periods divide fiscal year % one month at a time, in order', NEW.fiscal_year_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_ledger_record_closing_event() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  target finance_ledger_periods%ROWTYPE;
  year_end date;
  year_status text;
  period_status text;
  previous_status text;
  next_status text;
BEGIN
  PERFORM 1 FROM finance_ledger_books WHERE company_id = NEW.company_id FOR NO KEY UPDATE;
  NEW.event_number := COALESCE((SELECT max(event_number) FROM finance_ledger_closing_events
    WHERE company_id = NEW.company_id), 0) + 1;
  NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
  SELECT end_date INTO year_end FROM finance_ledger_fiscal_years
    WHERE id = NEW.fiscal_year_id AND company_id = NEW.company_id;
  IF NOT FOUND THEN RETURN NEW; END IF;
  year_status := finance_ledger_fiscal_year_status(NEW.fiscal_year_id);
  IF NEW.period_id IS NULL THEN
    IF NEW.action = 'close' AND year_status = 'closed' THEN
      RAISE EXCEPTION 'fiscal year % is already closed', NEW.fiscal_year_id USING ERRCODE = 'restrict_violation';
    ELSIF NEW.action = 'reopen' AND year_status <> 'closed' THEN
      RAISE EXCEPTION 'fiscal year % is not closed', NEW.fiscal_year_id USING ERRCODE = 'restrict_violation';
    ELSIF NEW.action = 'reopen' AND EXISTS (SELECT 1 FROM finance_ledger_periods
      WHERE fiscal_year_id = NEW.fiscal_year_id AND end_date = year_end AND finance_ledger_period_status(id) = 'locked') THEN
      RAISE EXCEPTION 'fiscal year % ends in a locked period and never reopens', NEW.fiscal_year_id
        USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
  END IF;
  -- Postings hold FOR SHARE on their period until commit.
  SELECT * INTO target FROM finance_ledger_periods
    WHERE id = NEW.period_id AND fiscal_year_id = NEW.fiscal_year_id FOR UPDATE;
  IF NOT FOUND THEN RETURN NEW; END IF;
  period_status := finance_ledger_period_status(target.id);
  SELECT finance_ledger_period_status(id) INTO previous_status FROM finance_ledger_periods
    WHERE company_id = NEW.company_id AND end_date = target.start_date - 1;
  SELECT finance_ledger_period_status(id) INTO next_status FROM finance_ledger_periods
    WHERE company_id = NEW.company_id AND start_date = target.end_date + 1;
  IF NEW.action = 'close' THEN
    IF period_status <> 'open' THEN
      RAISE EXCEPTION 'period % is not open', target.id USING ERRCODE = 'restrict_violation';
    ELSIF previous_status = 'open' THEN
      RAISE EXCEPTION 'period % cannot close before the period before it', target.id USING ERRCODE = 'restrict_violation';
    ELSIF target.end_date = year_end AND year_status <> 'closed' THEN
      RAISE EXCEPTION 'period % is its fiscal year''s last and closes with the year', target.id USING ERRCODE = 'restrict_violation';
    END IF;
  ELSIF NEW.action = 'reopen' THEN
    IF period_status <> 'closed' THEN
      RAISE EXCEPTION 'period % is %, not closed', target.id, period_status USING ERRCODE = 'restrict_violation';
    ELSIF year_status = 'closed' THEN
      RAISE EXCEPTION 'period % is in a closed fiscal year: reopen the year', target.id USING ERRCODE = 'restrict_violation';
    ELSIF next_status IS NOT NULL AND next_status <> 'open' THEN
      RAISE EXCEPTION 'period % cannot reopen while the period after it is %', target.id, next_status USING ERRCODE = 'restrict_violation';
    END IF;
  ELSIF NEW.action = 'lock' THEN
    IF period_status <> 'closed' THEN
      RAISE EXCEPTION 'period % is %, not closed', target.id, period_status USING ERRCODE = 'restrict_violation';
    ELSIF previous_status IS NOT NULL AND previous_status <> 'locked' THEN
      RAISE EXCEPTION 'period % cannot lock before the period before it', target.id USING ERRCODE = 'restrict_violation';
    END IF;
  END IF;
  RETURN NEW;
END
$$;
CREATE FUNCTION finance_ledger_check_closed_year() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF finance_ledger_fiscal_year_status(NEW.fiscal_year_id) = 'closed' AND EXISTS (
    SELECT 1 FROM finance_ledger_periods WHERE fiscal_year_id = NEW.fiscal_year_id
      AND finance_ledger_period_status(id) = 'open') THEN
    RAISE EXCEPTION 'fiscal year % is closed but has an open period', NEW.fiscal_year_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END
$$;
CREATE FUNCTION finance_ledger_refuse_closed_period() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  target bigint;
  status text;
BEGIN
  SELECT id INTO target FROM finance_ledger_periods WHERE company_id = NEW.company_id
    AND NEW.entry_date BETWEEN start_date AND end_date FOR SHARE;
  IF FOUND THEN
    status := finance_ledger_period_status(target);
    IF status <> 'open' THEN
      RAISE EXCEPTION 'journal entry dated % falls in a % period', NEW.entry_date, status USING ERRCODE = 'restrict_violation';
    END IF;
  ELSIF EXISTS (SELECT 1 FROM finance_ledger_periods WHERE company_id = NEW.company_id AND start_date > NEW.entry_date) THEN
    RAISE EXCEPTION 'journal entry dated % is before the first fiscal year', NEW.entry_date USING ERRCODE = 'restrict_violation';
  ELSIF finance_ledger_company_controlled(NEW.company_id) THEN
    RAISE EXCEPTION 'journal entry dated % falls in no fiscal period', NEW.entry_date USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END
$$;

CREATE TRIGGER finance_ledger_fiscal_years_append_only BEFORE UPDATE OR DELETE ON finance_ledger_fiscal_years FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_fiscal_years_no_truncate BEFORE TRUNCATE ON finance_ledger_fiscal_years FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_periods_append_only BEFORE UPDATE OR DELETE ON finance_ledger_periods FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_periods_no_truncate BEFORE TRUNCATE ON finance_ledger_periods FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_closing_events_append_only BEFORE UPDATE OR DELETE ON finance_ledger_closing_events FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_closing_events_no_truncate BEFORE TRUNCATE ON finance_ledger_closing_events FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_closing_change();
CREATE TRIGGER finance_ledger_fiscal_years_open BEFORE INSERT ON finance_ledger_fiscal_years FOR EACH ROW EXECUTE FUNCTION finance_ledger_open_fiscal_year();
CREATE CONSTRAINT TRIGGER finance_ledger_fiscal_years_divided AFTER INSERT ON finance_ledger_fiscal_years DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_fiscal_year_periods();
CREATE TRIGGER finance_ledger_periods_place BEFORE INSERT ON finance_ledger_periods FOR EACH ROW EXECUTE FUNCTION finance_ledger_place_period();
CREATE TRIGGER finance_ledger_closing_events_record BEFORE INSERT ON finance_ledger_closing_events FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_closing_event();
CREATE CONSTRAINT TRIGGER finance_ledger_closing_events_closed_year AFTER INSERT ON finance_ledger_closing_events DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_closed_year();
CREATE TRIGGER finance_ledger_entries_open_period BEFORE INSERT ON finance_ledger_entries FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_closed_period();

CREATE TABLE finance_ledger_late_recording_reviews (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  entry_id bigint NOT NULL,
  reason varchar(500) NOT NULL,
  reviewed_by_user_id bigint NOT NULL,
  reviewed_by_impersonator_id bigint,
  reviewed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_late_recording_reviews_entry_fkey FOREIGN KEY (entry_id, company_id)
    REFERENCES finance_ledger_entries (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_late_recording_reviews_reason CHECK (btrim(reason) <> '')
);
CREATE UNIQUE INDEX finance_ledger_late_recording_reviews_entry_unique ON finance_ledger_late_recording_reviews (entry_id);
CREATE INDEX finance_ledger_late_recording_reviews_tenant_company_index ON finance_ledger_late_recording_reviews (tenant_id, company_id);
CREATE FUNCTION finance_ledger_record_late_recording_review() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  entry finance_ledger_entries%ROWTYPE;
BEGIN
  SELECT * INTO entry FROM finance_ledger_entries WHERE id = NEW.entry_id;
  IF CAST(entry.posted_at AS date) - entry.entry_date < 45 THEN
    RAISE EXCEPTION 'journal entry % was not recorded late', NEW.entry_id USING ERRCODE = 'restrict_violation';
  END IF;
  IF NEW.reviewed_by_user_id IN (entry.posted_by_user_id, entry.posted_by_impersonator_id)
    OR NEW.reviewed_by_impersonator_id IN (entry.posted_by_user_id, entry.posted_by_impersonator_id) THEN
    RAISE EXCEPTION 'journal entry % was posted by the reviewer', NEW.entry_id USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.reviewed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_late_recording_reviews_append_only BEFORE UPDATE OR DELETE ON finance_ledger_late_recording_reviews FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_late_recording_reviews_no_truncate BEFORE TRUNCATE ON finance_ledger_late_recording_reviews FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_late_recording_reviews_record BEFORE INSERT ON finance_ledger_late_recording_reviews FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_late_recording_review();

CREATE TABLE finance_ledger_duty_separation_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  change_number bigint NOT NULL,
  required boolean NOT NULL,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_duty_separation_changes_change_number CHECK (change_number > 0),
  CONSTRAINT finance_ledger_duty_separation_changes_reason CHECK (required OR (reason IS NOT NULL AND btrim(reason) <> ''))
);
CREATE UNIQUE INDEX finance_ledger_duty_separation_changes_id_company_unique ON finance_ledger_duty_separation_changes (id, company_id);
CREATE UNIQUE INDEX finance_ledger_duty_separation_changes_company_number_unique ON finance_ledger_duty_separation_changes (company_id, change_number);
CREATE INDEX finance_ledger_duty_separation_changes_tenant_company_index ON finance_ledger_duty_separation_changes (tenant_id, company_id);
CREATE TABLE finance_ledger_approval_exceptions (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  exception_number bigint NOT NULL,
  source varchar(64) NOT NULL,
  action varchar(40) NOT NULL,
  subject_type varchar(40) NOT NULL,
  subject_id bigint NOT NULL,
  subject varchar(200) NOT NULL,
  same_person boolean NOT NULL,
  override boolean NOT NULL,
  prepared_by_user_id bigint,
  prepared_by_impersonator_id bigint,
  approved_by_user_id bigint NOT NULL,
  approved_by_impersonator_id bigint,
  reason varchar(500) NOT NULL,
  separation_change_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_approval_exceptions_separation_change_fkey FOREIGN KEY (separation_change_id, company_id)
    REFERENCES finance_ledger_duty_separation_changes (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_approval_exceptions_exception_number CHECK (exception_number > 0),
  CONSTRAINT finance_ledger_approval_exceptions_codes CHECK (source ~ '^[a-z][a-z0-9_/-]*$'
    AND action ~ '^[a-z][a-z0-9_]*$' AND subject_type ~ '^[a-z][a-z0-9_]*$'
    AND subject_id > 0 AND btrim(subject) <> '' AND btrim(reason) <> ''),
  CONSTRAINT finance_ledger_approval_exceptions_kind CHECK (same_person OR override),
  CONSTRAINT finance_ledger_approval_exceptions_same_person CHECK (
    same_person = (COALESCE(approved_by_user_id IN (prepared_by_user_id, prepared_by_impersonator_id), false)
      OR COALESCE(approved_by_impersonator_id IN (prepared_by_user_id, prepared_by_impersonator_id), false))
    AND same_person = (separation_change_id IS NOT NULL)),
  CONSTRAINT finance_ledger_approval_exceptions_preparer CHECK (prepared_by_user_id IS NOT NULL OR prepared_by_impersonator_id IS NULL)
);
CREATE UNIQUE INDEX finance_ledger_approval_exceptions_id_company_unique ON finance_ledger_approval_exceptions (id, company_id);
CREATE UNIQUE INDEX finance_ledger_approval_exceptions_company_number_unique ON finance_ledger_approval_exceptions (company_id, exception_number);
CREATE INDEX finance_ledger_approval_exceptions_subject_index ON finance_ledger_approval_exceptions (company_id, source, subject_type, subject_id);
CREATE INDEX finance_ledger_approval_exceptions_recorded_index ON finance_ledger_approval_exceptions (company_id, recorded_at);
CREATE INDEX finance_ledger_approval_exceptions_tenant_id_company_id_index ON finance_ledger_approval_exceptions (tenant_id, company_id);
CREATE TABLE finance_ledger_approval_exception_reviews (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  exception_id bigint NOT NULL,
  note varchar(500),
  reviewed_by_user_id bigint NOT NULL,
  reviewed_by_impersonator_id bigint,
  reviewed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_approval_exception_reviews_exception_fkey FOREIGN KEY (exception_id, company_id)
    REFERENCES finance_ledger_approval_exceptions (id, company_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX finance_ledger_approval_exception_reviews_exception_unique ON finance_ledger_approval_exception_reviews (exception_id);
CREATE INDEX finance_ledger_approval_exception_reviews_tenant_company_index ON finance_ledger_approval_exception_reviews (tenant_id, company_id);

CREATE FUNCTION finance_ledger_record_duty_separation_change() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  current boolean;
BEGIN
  SELECT required INTO current FROM finance_ledger_duty_separation_changes
    WHERE company_id = NEW.company_id ORDER BY change_number DESC LIMIT 1;
  IF COALESCE(current, true) = NEW.required THEN
    RAISE EXCEPTION 'duty separation of company % is already %', NEW.company_id,
      CASE WHEN NEW.required THEN 'required' ELSE 'not required' END USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.change_number := COALESCE((SELECT max(change_number) FROM finance_ledger_duty_separation_changes WHERE company_id = NEW.company_id), 0) + 1;
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE FUNCTION finance_ledger_record_approval_exception() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  latest finance_ledger_duty_separation_changes%ROWTYPE;
BEGIN
  IF NEW.same_person THEN
    SELECT * INTO latest FROM finance_ledger_duty_separation_changes
      WHERE company_id = NEW.company_id ORDER BY change_number DESC LIMIT 1;
    IF latest.id IS DISTINCT FROM NEW.separation_change_id OR latest.required THEN
      RAISE EXCEPTION 'company % requires separation of duties: the person who prepared % % cannot approve it',
        NEW.company_id, NEW.subject_type, NEW.subject_id USING ERRCODE = 'restrict_violation';
    END IF;
  END IF;
  NEW.exception_number := COALESCE((SELECT max(exception_number) FROM finance_ledger_approval_exceptions WHERE company_id = NEW.company_id), 0) + 1;
  NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE FUNCTION finance_ledger_record_approval_exception_review() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  reviewed finance_ledger_approval_exceptions%ROWTYPE;
BEGIN
  SELECT * INTO reviewed FROM finance_ledger_approval_exceptions WHERE id = NEW.exception_id AND company_id = NEW.company_id;
  IF FOUND AND (
    COALESCE(NEW.reviewed_by_user_id IN (reviewed.approved_by_user_id, reviewed.approved_by_impersonator_id), false)
    OR COALESCE(NEW.reviewed_by_impersonator_id IN (reviewed.approved_by_user_id, reviewed.approved_by_impersonator_id), false)
    OR COALESCE(NEW.reviewed_by_user_id IN (reviewed.prepared_by_user_id, reviewed.prepared_by_impersonator_id), false)
    OR COALESCE(NEW.reviewed_by_impersonator_id IN (reviewed.prepared_by_user_id, reviewed.prepared_by_impersonator_id), false)
  ) THEN
    RAISE EXCEPTION 'approval exception % is reviewed by someone who neither approved nor prepared it', NEW.exception_id USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.reviewed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_duty_separation_changes_append_only BEFORE UPDATE OR DELETE ON finance_ledger_duty_separation_changes FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_duty_separation_changes_no_truncate BEFORE TRUNCATE ON finance_ledger_duty_separation_changes FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_duty_separation_changes_record BEFORE INSERT ON finance_ledger_duty_separation_changes FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_duty_separation_change();
CREATE TRIGGER finance_ledger_approval_exceptions_append_only BEFORE UPDATE OR DELETE ON finance_ledger_approval_exceptions FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_approval_exceptions_no_truncate BEFORE TRUNCATE ON finance_ledger_approval_exceptions FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_approval_exceptions_record BEFORE INSERT ON finance_ledger_approval_exceptions FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_approval_exception();
CREATE TRIGGER finance_ledger_approval_exception_reviews_append_only BEFORE UPDATE OR DELETE ON finance_ledger_approval_exception_reviews FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_approval_exception_reviews_no_truncate BEFORE TRUNCATE ON finance_ledger_approval_exception_reviews FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_approval_exception_reviews_record BEFORE INSERT ON finance_ledger_approval_exception_reviews FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_approval_exception_review();
