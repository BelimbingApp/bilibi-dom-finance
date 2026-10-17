-- finance/ledger 002: journal entries and their lines, the end state of the
-- Bilimbi ledger's entry schema (b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9) without its legacy-entry claims,
-- which belong to Bilimbi's own upgrade path and are not ported.
--
-- Rows of one company reference each other through composite
-- (id, company_id) foreign keys, so a line cannot name another company's
-- account or entry, nor an entry reverse another company's. An entry and its
-- lines are append-only: a trigger refuses UPDATE, DELETE and TRUNCATE, so a
-- posted entry is corrected by a reversing entry. The database numbers each
-- company's entries without gaps and stamps their posting time. A line can
-- only be inserted in the transaction that inserts its entry, so a later
-- transaction cannot add lines to a posted entry. A deferred constraint
-- trigger refuses a commit in which an entry has no lines or its debits and
-- credits differ.
--
-- An entry may be owned by the module and record that recognise it
-- (`owner_module` is a descriptor id such as `finance/receivables`,
-- `owner_record` is `<kind>:<id>`); a manual entry has neither. An owned
-- record has one original entry per company, and a reversal carries exactly
-- its original's owner, so an owned entry is never reversed by a writer that
-- does not act for its owner.

CREATE TABLE finance_ledger_entries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  entry_number bigint NOT NULL,
  entry_date date NOT NULL,
  description varchar(500) NOT NULL,
  reference varchar(100),
  currency_code varchar(3) NOT NULL,
  reverses_entry_id bigint,
  owner_module varchar(100),
  owner_record varchar(100),
  posted_by_user_id bigint NOT NULL,
  posted_by_impersonator_id bigint,
  posted_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_entries_book_fkey
    FOREIGN KEY (currency_code, company_id)
    REFERENCES finance_ledger_books (currency_code, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_entries_entry_number CHECK (entry_number > 0),
  CONSTRAINT finance_ledger_entries_not_own_reversal
    CHECK (reverses_entry_id IS NULL OR reverses_entry_id <> id),
  CONSTRAINT finance_ledger_entries_owner_pair
    CHECK ((owner_module IS NULL) = (owner_record IS NULL)),
  CONSTRAINT finance_ledger_entries_owner_module
    CHECK (owner_module IS NULL OR owner_module ~ '^[a-z][a-z0-9_]*/[a-z][a-z0-9_]*$'),
  CONSTRAINT finance_ledger_entries_owner_record
    CHECK (owner_record IS NULL OR owner_record ~ '^[a-z][a-z0-9_]*:[1-9][0-9]*$')
);
CREATE UNIQUE INDEX finance_ledger_entries_id_company_unique ON finance_ledger_entries (id, company_id);
-- A self-reference needs the composite key above before it can exist.
ALTER TABLE finance_ledger_entries ADD CONSTRAINT finance_ledger_entries_reverses_entry_fkey
  FOREIGN KEY (reverses_entry_id, company_id) REFERENCES finance_ledger_entries (id, company_id)
  ON DELETE RESTRICT;
CREATE UNIQUE INDEX finance_ledger_entries_reversed_once ON finance_ledger_entries (reverses_entry_id)
  WHERE reverses_entry_id IS NOT NULL;
CREATE INDEX finance_ledger_entries_tenant_id_company_id_entry_date_index
  ON finance_ledger_entries (tenant_id, company_id, entry_date);
CREATE UNIQUE INDEX finance_ledger_entries_company_number_unique
  ON finance_ledger_entries (company_id, entry_number);
CREATE UNIQUE INDEX finance_ledger_entries_owner_record_unique
  ON finance_ledger_entries (company_id, owner_module, owner_record)
  WHERE owner_module IS NOT NULL AND reverses_entry_id IS NULL;

CREATE TABLE finance_ledger_entry_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  entry_id bigint NOT NULL,
  line_number integer NOT NULL,
  account_id bigint NOT NULL,
  debit numeric(20,4) NOT NULL,
  credit numeric(20,4) NOT NULL,
  description varchar(500),
  CONSTRAINT finance_ledger_entry_lines_entry_fkey
    FOREIGN KEY (entry_id, company_id) REFERENCES finance_ledger_entries (id, company_id)
    ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_entry_lines_account_fkey
    FOREIGN KEY (account_id, company_id) REFERENCES finance_ledger_accounts (id, company_id)
    ON DELETE RESTRICT,
  CONSTRAINT finance_ledger_entry_lines_line_number CHECK (line_number > 0),
  -- Exactly one side of a line carries a positive amount.
  CONSTRAINT finance_ledger_entry_lines_one_side
    CHECK (debit >= 0 AND credit >= 0 AND (debit = 0) <> (credit = 0))
);
CREATE UNIQUE INDEX finance_ledger_entry_lines_entry_line_unique
  ON finance_ledger_entry_lines (entry_id, line_number);
CREATE INDEX finance_ledger_entry_lines_company_id_account_id_index
  ON finance_ledger_entry_lines (company_id, account_id);

CREATE TRIGGER finance_ledger_entries_append_only
  BEFORE UPDATE OR DELETE ON finance_ledger_entries
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_entry_lines_append_only
  BEFORE UPDATE OR DELETE ON finance_ledger_entry_lines
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_entries_no_truncate
  BEFORE TRUNCATE ON finance_ledger_entries
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_entry_lines_no_truncate
  BEFORE TRUNCATE ON finance_ledger_entry_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_change();

-- The database numbers each company's entries 1, 2, 3, ... and stamps the
-- posting time, whatever the inserting writer sent: with no sequence and no
-- deletion, a number is never skipped or reused, and a concurrent writer that
-- takes the same number fails on the unique index (the facade also holds the
-- company's row lock, so writers queue instead).
CREATE FUNCTION finance_ledger_number_entry() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.entry_number := COALESCE(
    (SELECT max(entry_number) FROM finance_ledger_entries
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.posted_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_entries_number
  BEFORE INSERT ON finance_ledger_entries
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_number_entry();

-- A reversal names its original's owner, or none when the original has none:
-- the owner's correction path posts it, nothing else does.
CREATE FUNCTION finance_ledger_check_reversal_owner() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  original finance_ledger_entries%ROWTYPE;
BEGIN
  IF NEW.reverses_entry_id IS NULL THEN
    RETURN NEW;
  END IF;

  SELECT * INTO original FROM finance_ledger_entries
  WHERE id = NEW.reverses_entry_id AND company_id = NEW.company_id;

  IF NOT FOUND THEN
    RETURN NEW;
  END IF;

  IF original.owner_module IS NOT NULL
    AND (NEW.owner_module, NEW.owner_record)
        IS DISTINCT FROM (original.owner_module, original.owner_record) THEN
    RAISE EXCEPTION 'journal entry % is owned by % (%): it is reversed through that module',
      original.id, original.owner_module, original.owner_record
      USING ERRCODE = 'restrict_violation';
  ELSIF original.owner_module IS NULL AND NEW.owner_module IS NOT NULL THEN
    RAISE EXCEPTION 'journal entry % has no owner: its reversal has none either', original.id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_entries_reversal_owner
  BEFORE INSERT ON finance_ledger_entries
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_reversal_owner();

-- An entry is "posted" once its inserting transaction is no longer in
-- progress. pg_xact_status is NULL for a transaction older than the retained
-- commit log, so IS DISTINCT FROM keeps old entries refused.
CREATE FUNCTION finance_ledger_refuse_posted_line() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_ledger_entries
    WHERE id = NEW.entry_id
      AND pg_xact_status(
            ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
          ) IS DISTINCT FROM 'in progress'
  ) THEN
    RAISE EXCEPTION 'journal entry % is already posted: a line cannot be added; post a reversing entry',
      NEW.entry_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_entry_lines_posted_entry
  BEFORE INSERT ON finance_ledger_entry_lines
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_posted_line();

-- Checked once per entry at commit, whichever of its rows is inserted last.
CREATE FUNCTION finance_ledger_check_balance() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
BEGIN
  IF TG_TABLE_NAME = 'finance_ledger_entries' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.entry_id;
  END IF;

  IF NOT EXISTS (SELECT 1 FROM finance_ledger_entry_lines WHERE entry_id = checked_id) THEN
    RAISE EXCEPTION 'journal entry % has no lines', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF (SELECT sum(debit) - sum(credit) FROM finance_ledger_entry_lines
      WHERE entry_id = checked_id) <> 0 THEN
    RAISE EXCEPTION 'journal entry % does not balance', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;
CREATE CONSTRAINT TRIGGER finance_ledger_entries_balanced
  AFTER INSERT ON finance_ledger_entries
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_balance();
CREATE CONSTRAINT TRIGGER finance_ledger_entry_lines_balanced
  AFTER INSERT ON finance_ledger_entry_lines
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_check_balance();
