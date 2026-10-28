-- finance/ledger 006: withdrawing a submitted manual journal.
--
-- Port addition (Bilimbi has no withdrawal). Until approval nothing posts, so
-- the person who submitted a journal may take it back to correct it. The
-- withdrawal is the journal's one decision, like an approval or a rejection:
-- a third event, `withdrawn`, appended by whoever submitted the journal (the
-- database refuses anyone else), so the submission stays on record with its
-- lines, who withdrew it and when, and an approval racing the withdrawal loses
-- to 005's unique decision per journal (and to the company lock). Nothing a
-- journal holds is ever edited: the corrected journal is a new journal, which
-- names the one it replaces in finance_ledger_journal_replacements, and it
-- needs its own approval.
--
-- The link is its own append-only table rather than a column on the journals
-- table or the events table, whose evidence field lists are frozen.

ALTER TABLE finance_ledger_journal_events DROP CONSTRAINT finance_ledger_journal_events_event;
ALTER TABLE finance_ledger_journal_events ADD CONSTRAINT finance_ledger_journal_events_event
  CHECK (event IN ('approved', 'rejected', 'withdrawn'));

-- 005's guard, plus: only whoever submitted a journal withdraws it.
CREATE OR REPLACE FUNCTION finance_ledger_record_journal_event() RETURNS trigger
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
  IF NEW.event = 'withdrawn' AND FOUND AND NEW.actor_user_id <> journal.submitted_by_user_id THEN
    RAISE EXCEPTION 'journal % is withdrawn by someone other than whoever submitted it', NEW.journal_id
      USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE TABLE finance_ledger_journal_replacements (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  journal_id bigint NOT NULL,
  replaced_journal_id bigint NOT NULL,
  CONSTRAINT finance_ledger_journal_replacements_journal_fkey FOREIGN KEY (journal_id, company_id)
    REFERENCES finance_ledger_journals (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_replacements_replaced_fkey FOREIGN KEY (replaced_journal_id, company_id)
    REFERENCES finance_ledger_journals (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_journal_replacements_distinct CHECK (journal_id <> replaced_journal_id)
);
-- A journal replaces at most one journal, and a journal is replaced at most once.
CREATE UNIQUE INDEX finance_ledger_journal_replacements_journal_unique ON finance_ledger_journal_replacements (journal_id);
CREATE UNIQUE INDEX finance_ledger_journal_replacements_replaced_unique ON finance_ledger_journal_replacements (replaced_journal_id);
CREATE INDEX finance_ledger_journal_replacements_tenant_id_company_id_index ON finance_ledger_journal_replacements (tenant_id, company_id);

-- A replacement is recorded in the transaction that withdrew the journal it
-- replaces, for the same submitter, and the replacement is the later journal.
CREATE FUNCTION finance_ledger_check_journal_replacement() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  replacement finance_ledger_journals%ROWTYPE;
  replaced finance_ledger_journals%ROWTYPE;
BEGIN
  SELECT * INTO replacement FROM finance_ledger_journals WHERE id = NEW.journal_id AND company_id = NEW.company_id;
  SELECT * INTO replaced FROM finance_ledger_journals WHERE id = NEW.replaced_journal_id AND company_id = NEW.company_id;
  IF replacement.submitted_by_user_id <> replaced.submitted_by_user_id
    OR replacement.journal_number <= replaced.journal_number THEN
    RAISE EXCEPTION 'journal % does not replace journal %', NEW.journal_id, NEW.replaced_journal_id
      USING ERRCODE = 'check_violation';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM finance_ledger_journal_events
    WHERE journal_id = NEW.replaced_journal_id AND company_id = NEW.company_id AND event = 'withdrawn'
      AND actor_user_id = replaced.submitted_by_user_id
      AND pg_xact_status(
            ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
          ) = 'in progress'
  ) THEN
    RAISE EXCEPTION 'journal % is replaced only in the transaction that withdraws it', NEW.replaced_journal_id
      USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NULL;
END
$$;

CREATE TRIGGER finance_ledger_journal_replacements_append_only BEFORE UPDATE OR DELETE ON finance_ledger_journal_replacements FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_journal_replacements_no_truncate BEFORE TRUNCATE ON finance_ledger_journal_replacements FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE CONSTRAINT TRIGGER finance_ledger_journal_replacements_check AFTER INSERT ON finance_ledger_journal_replacements DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_journal_replacement();
