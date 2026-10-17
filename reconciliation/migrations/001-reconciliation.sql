-- finance/reconciliation 001: bank accounts, imported statements, matches and
-- reconciliations, the end state of the Bilimbi reconciliation schema
-- (b-dom-finance 4abfaf829321f8540581d833752a086b21b45ba9, migrations
-- 20261014090000_create_finance_reconciliation and
-- 20261017090100_add_finance_reconciliation_import_mapping).
--
-- Bank reconciliation: each company's bank accounts, mapped one-to-one to a
-- ledger account and carrying the column mapping their statements import
-- with; the imported statements and their lines; the matches between
-- statement lines and ledger entries, with the events that make and unmake
-- them; and the reconciliations of an account as of a statement end date,
-- with the events that open, finalise, or abandon them and the snapshot of
-- items a finalised one explained its difference with.
--
-- Rows of one company reference each other through composite
-- (id, company_id) foreign keys. Ledger account and entry ids are plain
-- columns: the ledger's tables belong to finance/ledger, which validates every
-- account and posts every entry when the facade calls it.
--
-- A bank account keeps its company and ledger account; every other change to
-- it is recorded in the same transaction with who made it, why, and the
-- values before and after. A statement and its lines are inserted in one
-- transaction and never changed; the same file (by SHA-256) and the same line
-- (by fingerprint) are refused a second time. A match and its lines and
-- entries are inserted in one transaction with its `matched` event; a
-- deferred trigger refuses a commit in which the statement side does not
-- equal the ledger side; it is undone only by an `unmatched` event with a
-- reason, and never once a finalised reconciliation counted it. A
-- reconciliation is opened, then finalised or abandoned, each an append-only
-- event; a finalised one records the figures it was finalised with and the
-- entry, statement, and match-event numbers that reproduce them, and its
-- items are inserted only in that transaction. The database numbers each
-- company's statements, matches, match events, reconciliations, and
-- reconciliation events without gaps and stamps their times. Nothing is ever
-- deleted or truncated.
--
-- A fresh Bilibi schema has no statement imported before the mapping was
-- kept, so a statement's `import_mapping` is NOT NULL here (Bilimbi's column
-- is nullable for those).

-- One bank account per ledger account. The mapping says which CSV
-- columns a statement's date, description, reference, amount (or
-- debit and credit), and running balance are read from.
CREATE TABLE finance_reconciliation_bank_accounts (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  name varchar(200) NOT NULL,
  ledger_account_id bigint NOT NULL,
  charges_account_id bigint,
  interest_account_id bigint,
  match_window_days smallint NOT NULL DEFAULT 5,
  import_mapping jsonb NOT NULL,
  active boolean NOT NULL DEFAULT true,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_reconciliation_bank_accounts_match_window_days
    CHECK (match_window_days BETWEEN 0 AND 90)
);

CREATE UNIQUE INDEX finance_reconciliation_bank_accounts_id_company_unique
  ON finance_reconciliation_bank_accounts (id, company_id);

CREATE UNIQUE INDEX finance_reconciliation_bank_accounts_ledger_account_unique
  ON finance_reconciliation_bank_accounts (company_id, ledger_account_id);

CREATE UNIQUE INDEX finance_reconciliation_bank_accounts_company_name_unique
  ON finance_reconciliation_bank_accounts (company_id, name);

CREATE INDEX finance_reconciliation_bank_accounts_tenant_id_company_id_index
  ON finance_reconciliation_bank_accounts (tenant_id, company_id);

-- A bank account changes in place, so each change also lands here,
-- never altered: the field, the values before and after, the actor,
-- the reason, and the database's time.
CREATE TABLE finance_reconciliation_bank_account_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  field varchar(50) NOT NULL,
  old_value text,
  new_value text,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_bank_account_changes_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT
);

CREATE INDEX finance_reconciliation_bank_account_changes_bank_account_index
  ON finance_reconciliation_bank_account_changes (company_id, bank_account_id);

CREATE TABLE finance_reconciliation_statements (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  statement_number bigint NOT NULL,
  file_name varchar(255) NOT NULL,
  file_sha256 varchar(64) NOT NULL,
  import_mapping jsonb NOT NULL,
  line_count integer NOT NULL,
  start_date date NOT NULL,
  end_date date NOT NULL,
  opening_balance numeric(20, 4),
  closing_balance numeric(20, 4),
  imported_by_user_id bigint NOT NULL,
  imported_by_impersonator_id bigint,
  imported_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_statements_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_statements_statement_number
    CHECK (statement_number > 0),
  CONSTRAINT finance_reconciliation_statements_file_sha256
    CHECK (file_sha256 ~ '^[0-9a-f]{64}$'),
  CONSTRAINT finance_reconciliation_statements_line_count CHECK (line_count > 0),
  CONSTRAINT finance_reconciliation_statements_dates CHECK (end_date >= start_date),
  CONSTRAINT finance_reconciliation_statements_balances
    CHECK ((opening_balance IS NULL) = (closing_balance IS NULL))
);

CREATE UNIQUE INDEX finance_reconciliation_statements_id_company_unique
  ON finance_reconciliation_statements (id, company_id);

CREATE UNIQUE INDEX finance_reconciliation_statements_company_number_unique
  ON finance_reconciliation_statements (company_id, statement_number);

-- The same file is imported once.
CREATE UNIQUE INDEX finance_reconciliation_statements_file_unique
  ON finance_reconciliation_statements (company_id, file_sha256);

CREATE INDEX finance_reconciliation_statements_bank_account_index
  ON finance_reconciliation_statements (company_id, bank_account_id, end_date);

-- `amount` is signed from the account holder's side: money in is
-- positive, money out negative, which is the sign of a debit less a
-- credit on the asset account it is matched against. The fingerprint
-- hashes the line's values and its occurrence among identical lines
-- of the same file, so the same line is imported once while a
-- statement may carry two identical movements on one day.
CREATE TABLE finance_reconciliation_statement_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  statement_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  line_number integer NOT NULL,
  line_date date NOT NULL,
  description varchar(500) NOT NULL,
  reference varchar(100),
  amount numeric(20, 4) NOT NULL,
  running_balance numeric(20, 4),
  fingerprint varchar(64) NOT NULL,
  CONSTRAINT finance_reconciliation_statement_lines_statement_fkey
    FOREIGN KEY (statement_id, company_id)
    REFERENCES finance_reconciliation_statements (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_statement_lines_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_statement_lines_line_number CHECK (line_number > 0),
  CONSTRAINT finance_reconciliation_statement_lines_amount CHECK (amount <> 0),
  CONSTRAINT finance_reconciliation_statement_lines_fingerprint
    CHECK (fingerprint ~ '^[0-9a-f]{64}$')
);

CREATE UNIQUE INDEX finance_reconciliation_statement_lines_id_company_unique
  ON finance_reconciliation_statement_lines (id, company_id);

CREATE UNIQUE INDEX finance_reconciliation_statement_lines_statement_line_unique
  ON finance_reconciliation_statement_lines (statement_id, line_number);

-- The same line is imported once per bank account.
CREATE UNIQUE INDEX finance_reconciliation_statement_lines_fingerprint_unique
  ON finance_reconciliation_statement_lines (company_id, bank_account_id, fingerprint);

CREATE INDEX finance_reconciliation_statement_lines_bank_account_date_index
  ON finance_reconciliation_statement_lines (company_id, bank_account_id, line_date);

-- A match groups any number of statement lines with any number of
-- ledger entries whose amounts agree. It is made whole in one
-- transaction and undone only by an `unmatched` event.
CREATE TABLE finance_reconciliation_matches (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  match_number bigint NOT NULL,
  note varchar(500),
  matched_by_user_id bigint NOT NULL,
  matched_by_impersonator_id bigint,
  matched_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_matches_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_matches_match_number CHECK (match_number > 0)
);

CREATE UNIQUE INDEX finance_reconciliation_matches_id_company_unique
  ON finance_reconciliation_matches (id, company_id);

CREATE UNIQUE INDEX finance_reconciliation_matches_company_number_unique
  ON finance_reconciliation_matches (company_id, match_number);

CREATE INDEX finance_reconciliation_matches_bank_account_index
  ON finance_reconciliation_matches (company_id, bank_account_id);

CREATE TABLE finance_reconciliation_match_lines (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  match_id bigint NOT NULL,
  statement_line_id bigint NOT NULL,
  CONSTRAINT finance_reconciliation_match_lines_match_fkey
    FOREIGN KEY (match_id, company_id)
    REFERENCES finance_reconciliation_matches (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_match_lines_statement_line_fkey
    FOREIGN KEY (statement_line_id, company_id)
    REFERENCES finance_reconciliation_statement_lines (id, company_id) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX finance_reconciliation_match_lines_match_line_unique
  ON finance_reconciliation_match_lines (match_id, statement_line_id);

CREATE INDEX finance_reconciliation_match_lines_statement_line_index
  ON finance_reconciliation_match_lines (statement_line_id);

-- A ledger entry is named by ID and number; `amount` is its debits
-- less its credits on the bank account's ledger account when matched,
-- which never changes because posted entries never do. The entry's
-- date, description, and reference are copied for the same reason,
-- so a match and a finalised reconciliation read without the ledger.
CREATE TABLE finance_reconciliation_match_entries (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  match_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  entry_id bigint NOT NULL,
  entry_number bigint NOT NULL,
  entry_date date NOT NULL,
  description varchar(500) NOT NULL,
  reference varchar(100),
  amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_reconciliation_match_entries_match_fkey
    FOREIGN KEY (match_id, company_id)
    REFERENCES finance_reconciliation_matches (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_match_entries_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_match_entries_amount CHECK (amount <> 0)
);

CREATE UNIQUE INDEX finance_reconciliation_match_entries_match_entry_unique
  ON finance_reconciliation_match_entries (match_id, entry_id);

CREATE INDEX finance_reconciliation_match_entries_entry_index
  ON finance_reconciliation_match_entries (company_id, bank_account_id, entry_id);

-- A match has one `matched` event, recorded with it, and at most one
-- `unmatched` event, which needs a reason. The latest decides whether
-- the match stands.
CREATE TABLE finance_reconciliation_match_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  event_number bigint NOT NULL,
  match_id bigint NOT NULL,
  action varchar(16) NOT NULL,
  reason varchar(500),
  recorded_by_user_id bigint NOT NULL,
  recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_match_events_match_fkey
    FOREIGN KEY (match_id, company_id)
    REFERENCES finance_reconciliation_matches (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_match_events_action
    CHECK (action IN ('matched', 'unmatched')),
  CONSTRAINT finance_reconciliation_match_events_event_number CHECK (event_number > 0),
  CONSTRAINT finance_reconciliation_match_events_unmatch_reason
    CHECK (action <> 'unmatched' OR (reason IS NOT NULL AND btrim(reason) <> ''))
);

CREATE UNIQUE INDEX finance_reconciliation_match_events_company_number_unique
  ON finance_reconciliation_match_events (company_id, event_number);

CREATE UNIQUE INDEX finance_reconciliation_match_events_match_action_unique
  ON finance_reconciliation_match_events (match_id, action);

-- A reconciliation of one bank account as of a statement end date,
-- with the balance the statement shows on that date.
CREATE TABLE finance_reconciliation_reconciliations (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  reconciliation_number bigint NOT NULL,
  end_date date NOT NULL,
  statement_balance numeric(20, 4) NOT NULL,
  opened_by_user_id bigint NOT NULL,
  opened_by_impersonator_id bigint,
  opened_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_reconciliations_bank_account_fkey
    FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_reconciliations_reconciliation_number
    CHECK (reconciliation_number > 0)
);

CREATE UNIQUE INDEX finance_reconciliation_reconciliations_id_company_unique
  ON finance_reconciliation_reconciliations (id, company_id);

CREATE UNIQUE INDEX finance_reconciliation_reconciliations_company_number_unique
  ON finance_reconciliation_reconciliations (company_id, reconciliation_number);

CREATE INDEX finance_reconciliation_reconciliations_bank_account_index
  ON finance_reconciliation_reconciliations (company_id, bank_account_id, end_date);

-- `opened` is recorded with the reconciliation; `finalised` carries
-- the book balance, the difference (zero), and the entry, statement,
-- and match-event numbers the figures were read at; `abandoned` needs
-- a reason. A reconciliation has at most one of each.
CREATE TABLE finance_reconciliation_reconciliation_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  event_number bigint NOT NULL,
  reconciliation_id bigint NOT NULL,
  action varchar(16) NOT NULL,
  reason varchar(500),
  book_balance numeric(20, 4),
  difference numeric(20, 4),
  through_entry_number bigint,
  through_statement_number bigint,
  through_match_event_number bigint,
  recorded_by_user_id bigint NOT NULL,
  recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL,
  CONSTRAINT finance_reconciliation_reconciliation_events_parent_fkey
    FOREIGN KEY (reconciliation_id, company_id)
    REFERENCES finance_reconciliation_reconciliations (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_reconciliation_events_action
    CHECK (action IN ('opened', 'finalised', 'abandoned')),
  CONSTRAINT finance_reconciliation_reconciliation_events_event_number
    CHECK (event_number > 0),
  CONSTRAINT finance_reconciliation_reconciliation_events_abandon_reason
    CHECK (action <> 'abandoned' OR (reason IS NOT NULL AND btrim(reason) <> '')),
  CONSTRAINT finance_reconciliation_reconciliation_events_figures
    CHECK ((action = 'finalised') = (book_balance IS NOT NULL)
      AND (action = 'finalised') = (difference IS NOT NULL)
      AND (action = 'finalised') = (through_entry_number IS NOT NULL)
      AND (action = 'finalised') = (through_statement_number IS NOT NULL)
      AND (action = 'finalised') = (through_match_event_number IS NOT NULL)
      AND (difference IS NULL OR difference = 0))
);

CREATE UNIQUE INDEX finance_reconciliation_reconciliation_events_number_unique
  ON finance_reconciliation_reconciliation_events (company_id, event_number);

CREATE UNIQUE INDEX finance_reconciliation_reconciliation_events_action_unique
  ON finance_reconciliation_reconciliation_events (reconciliation_id, action);

-- What a finalised reconciliation explained its difference with: the
-- statement lines not in the books, the entries not on the statement,
-- and the matches whose two sides fall on either side of the end date.
CREATE TABLE finance_reconciliation_reconciliation_items (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  reconciliation_id bigint NOT NULL,
  item_number integer NOT NULL,
  kind varchar(32) NOT NULL,
  statement_line_id bigint,
  entry_id bigint,
  match_id bigint,
  item_date date NOT NULL,
  description varchar(500) NOT NULL,
  reference varchar(100),
  amount numeric(20, 4) NOT NULL,
  CONSTRAINT finance_reconciliation_reconciliation_items_reconciliation_fkey
    FOREIGN KEY (reconciliation_id, company_id)
    REFERENCES finance_reconciliation_reconciliations (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_reconciliation_reconciliation_items_item_number
    CHECK (item_number > 0),
  CONSTRAINT finance_reconciliation_reconciliation_items_kind
    CHECK (kind IN ('statement_line', 'entry', 'timing')),
  CONSTRAINT finance_reconciliation_reconciliation_items_subject
    CHECK ((kind = 'statement_line') = (statement_line_id IS NOT NULL)
      AND (kind = 'entry') = (entry_id IS NOT NULL)
      AND (kind = 'timing') = (match_id IS NOT NULL))
);

CREATE UNIQUE INDEX finance_reconciliation_reconciliation_items_number_unique
  ON finance_reconciliation_reconciliation_items (reconciliation_id, item_number);

CREATE FUNCTION finance_reconciliation_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance reconciliation keeps % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

-- A bank account keeps its company and its ledger account: every match
-- and reconciliation of it reads that account's postings.
CREATE FUNCTION finance_reconciliation_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR NEW.ledger_account_id <> OLD.ledger_account_id THEN
    RAISE EXCEPTION 'a bank account keeps its company and ledger account'
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- Numbers a company's rows of each kind 1, 2, 3, ... in insertion order
-- and stamps the row's time, whatever the writer sent. With no sequence
-- and no deletion a number is never skipped or reused, and a concurrent
-- writer that takes the same number fails on the unique index.
CREATE FUNCTION finance_reconciliation_number() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  stamp timestamp := clock_timestamp() AT TIME ZONE 'UTC';
BEGIN
  IF TG_TABLE_NAME = 'finance_reconciliation_statements' THEN
    NEW.statement_number := COALESCE(
      (SELECT max(statement_number) FROM finance_reconciliation_statements
       WHERE company_id = NEW.company_id), 0) + 1;
    NEW.imported_at := stamp;
  ELSIF TG_TABLE_NAME = 'finance_reconciliation_matches' THEN
    NEW.match_number := COALESCE(
      (SELECT max(match_number) FROM finance_reconciliation_matches
       WHERE company_id = NEW.company_id), 0) + 1;
    NEW.matched_at := stamp;
  ELSIF TG_TABLE_NAME = 'finance_reconciliation_match_events' THEN
    NEW.event_number := COALESCE(
      (SELECT max(event_number) FROM finance_reconciliation_match_events
       WHERE company_id = NEW.company_id), 0) + 1;
    NEW.recorded_at := stamp;
  ELSIF TG_TABLE_NAME = 'finance_reconciliation_reconciliations' THEN
    NEW.reconciliation_number := COALESCE(
      (SELECT max(reconciliation_number) FROM finance_reconciliation_reconciliations
       WHERE company_id = NEW.company_id), 0) + 1;
    NEW.opened_at := stamp;
  ELSIF TG_TABLE_NAME = 'finance_reconciliation_reconciliation_events' THEN
    NEW.event_number := COALESCE(
      (SELECT max(event_number) FROM finance_reconciliation_reconciliation_events
       WHERE company_id = NEW.company_id), 0) + 1;
    NEW.recorded_at := stamp;
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_reconciliation_stamp_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

-- A statement's lines, and a match's lines and entries, are inserted
-- only in the transaction that inserts their parent.
CREATE FUNCTION finance_reconciliation_refuse_closed_part() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  open boolean;
  parent text;
BEGIN
  IF TG_TABLE_NAME = 'finance_reconciliation_statement_lines' THEN
    parent := 'statement ' || NEW.statement_id;
    SELECT pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
  ) = 'in progress' INTO open
    FROM finance_reconciliation_statements WHERE id = NEW.statement_id;
  ELSE
    parent := 'match ' || NEW.match_id;
    SELECT pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
  ) = 'in progress' INTO open
    FROM finance_reconciliation_matches WHERE id = NEW.match_id;
  END IF;

  IF NOT COALESCE(open, false) THEN
    RAISE EXCEPTION '% is already recorded: a % row cannot be added', parent, TG_TABLE_NAME
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- At commit, a statement has exactly the lines it counts, every line
-- on its bank account, and its dates span them.
CREATE FUNCTION finance_reconciliation_check_statement() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
  statement record;
BEGIN
  IF TG_TABLE_NAME = 'finance_reconciliation_statements' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.statement_id;

    -- Every line fires this at commit, and the statement's own row fires it
    -- too, so the checks below run once for the statement and once more for
    -- the line numbered last, not once per line: a statement of 10,000 lines
    -- would otherwise count its lines 10,000 times. A line numbered past the
    -- count is still caught by the statement's own check.
    IF NEW.line_number <> (SELECT line_count FROM finance_reconciliation_statements
                           WHERE id = checked_id) THEN
      RETURN NULL;
    END IF;
  END IF;

  SELECT * INTO statement FROM finance_reconciliation_statements WHERE id = checked_id;

  IF statement.line_count <> (SELECT count(*) FROM finance_reconciliation_statement_lines
                              WHERE statement_id = checked_id) THEN
    RAISE EXCEPTION 'statement % counts % lines but holds a different number',
      checked_id, statement.line_count
      USING ERRCODE = 'check_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_statement_lines
    WHERE statement_id = checked_id
      AND (bank_account_id <> statement.bank_account_id
           OR line_date < statement.start_date OR line_date > statement.end_date)
  ) THEN
    RAISE EXCEPTION 'statement % has a line outside its bank account or dates', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

-- A statement line is in at most one standing match: one with a
-- `matched` event and no `unmatched` event.
CREATE FUNCTION finance_reconciliation_refuse_taken_line() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_match_lines l
    WHERE l.statement_line_id = NEW.statement_line_id AND l.match_id <> NEW.match_id
      AND EXISTS (SELECT 1 FROM finance_reconciliation_match_events e
                  WHERE e.match_id = l.match_id AND e.action = 'matched')
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events e
                      WHERE e.match_id = l.match_id AND e.action = 'unmatched')
  ) THEN
    RAISE EXCEPTION 'statement line % is already matched', NEW.statement_line_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- A ledger entry is in at most one standing match of a bank account; a
-- transfer between two of the company's bank accounts is matched on
-- each.
CREATE FUNCTION finance_reconciliation_refuse_taken_entry() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_match_entries m
    WHERE m.bank_account_id = NEW.bank_account_id AND m.entry_id = NEW.entry_id
      AND m.match_id <> NEW.match_id
      AND EXISTS (SELECT 1 FROM finance_reconciliation_match_events e
                  WHERE e.match_id = m.match_id AND e.action = 'matched')
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events e
                      WHERE e.match_id = m.match_id AND e.action = 'unmatched')
  ) THEN
    RAISE EXCEPTION 'ledger entry % is already matched on bank account %',
      NEW.entry_id, NEW.bank_account_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- At commit, a match has at least one line and one entry on its bank
-- account, its `matched` event, and a statement side equal to its
-- ledger side.
CREATE FUNCTION finance_reconciliation_check_match() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  checked_id bigint;
  match record;
  statement_side numeric;
  ledger_side numeric;
BEGIN
  IF TG_TABLE_NAME = 'finance_reconciliation_matches' THEN
    checked_id := NEW.id;
  ELSE
    checked_id := NEW.match_id;
  END IF;

  SELECT * INTO match FROM finance_reconciliation_matches WHERE id = checked_id;

  SELECT sum(s.amount) INTO statement_side
  FROM finance_reconciliation_match_lines l
  JOIN finance_reconciliation_statement_lines s ON s.id = l.statement_line_id
  WHERE l.match_id = checked_id AND s.bank_account_id = match.bank_account_id;

  SELECT sum(amount) INTO ledger_side
  FROM finance_reconciliation_match_entries
  WHERE match_id = checked_id AND bank_account_id = match.bank_account_id;

  IF statement_side IS NULL OR ledger_side IS NULL THEN
    RAISE EXCEPTION 'match % needs a statement line and a ledger entry of its bank account',
      checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF statement_side <> ledger_side THEN
    RAISE EXCEPTION 'match % statement side % differs from ledger side %',
      checked_id, statement_side, ledger_side
      USING ERRCODE = 'check_violation';
  END IF;

  IF NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events
                 WHERE match_id = checked_id AND action = 'matched') THEN
    RAISE EXCEPTION 'match % has no matched event', checked_id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

-- A `matched` event is recorded with its match; an `unmatched` event
-- needs a standing match that no finalised reconciliation of the bank
-- account counted (its `matched` event is numbered after the
-- reconciliation's match-event number).
CREATE FUNCTION finance_reconciliation_record_match_event() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  match record;
  matched_number bigint;
BEGIN
  SELECT * INTO match FROM finance_reconciliation_matches WHERE id = NEW.match_id;

  IF NEW.action = 'matched' AND NOT (
    SELECT pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
  ) = 'in progress' FROM finance_reconciliation_matches
    WHERE id = NEW.match_id
  ) THEN
    RAISE EXCEPTION 'match % is already recorded: it is not matched again', NEW.match_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.action = 'unmatched' THEN
    SELECT event_number INTO matched_number FROM finance_reconciliation_match_events
    WHERE match_id = NEW.match_id AND action = 'matched';

    IF matched_number IS NULL THEN
      RAISE EXCEPTION 'match % is not matched', NEW.match_id
        USING ERRCODE = 'restrict_violation';
    END IF;

    IF EXISTS (
      SELECT 1 FROM finance_reconciliation_reconciliation_events e
      JOIN finance_reconciliation_reconciliations r ON r.id = e.reconciliation_id
      WHERE r.bank_account_id = match.bank_account_id AND e.action = 'finalised'
        AND e.through_match_event_number >= matched_number
    ) THEN
      RAISE EXCEPTION 'match % was counted by a finalised reconciliation and is kept',
        NEW.match_id
        USING ERRCODE = 'restrict_violation';
    END IF;
  END IF;

  RETURN NEW;
END
$$;

-- One reconciliation of a bank account is open at a time, and each
-- ends after the last finalised one.
CREATE FUNCTION finance_reconciliation_open_reconciliation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_reconciliations r
    WHERE r.bank_account_id = NEW.bank_account_id AND r.id <> NEW.id
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_reconciliation_events e
                      WHERE e.reconciliation_id = r.id
                        AND e.action IN ('finalised', 'abandoned'))
  ) THEN
    RAISE EXCEPTION 'bank account % already has an open reconciliation', NEW.bank_account_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_reconciliations r
    JOIN finance_reconciliation_reconciliation_events e ON e.reconciliation_id = r.id
    WHERE r.bank_account_id = NEW.bank_account_id AND e.action = 'finalised'
      AND r.end_date >= NEW.end_date
  ) THEN
    RAISE EXCEPTION 'bank account % is already reconciled through %', NEW.bank_account_id,
      (SELECT max(r.end_date) FROM finance_reconciliation_reconciliations r
       JOIN finance_reconciliation_reconciliation_events e ON e.reconciliation_id = r.id
       WHERE r.bank_account_id = NEW.bank_account_id AND e.action = 'finalised')
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- `opened` is recorded with its reconciliation; `finalised` and
-- `abandoned` need an open one.
CREATE FUNCTION finance_reconciliation_record_reconciliation_event() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.action = 'opened' AND NOT (
    SELECT pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
  ) = 'in progress' FROM finance_reconciliation_reconciliations
    WHERE id = NEW.reconciliation_id
  ) THEN
    RAISE EXCEPTION 'reconciliation % is already recorded: it is not opened again',
      NEW.reconciliation_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF NEW.action <> 'opened' AND EXISTS (
    SELECT 1 FROM finance_reconciliation_reconciliation_events
    WHERE reconciliation_id = NEW.reconciliation_id AND action IN ('finalised', 'abandoned')
  ) THEN
    RAISE EXCEPTION 'reconciliation % is already closed', NEW.reconciliation_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- At commit, every reconciliation has its `opened` event.
CREATE FUNCTION finance_reconciliation_check_reconciliation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM finance_reconciliation_reconciliation_events
                 WHERE reconciliation_id = NEW.id AND action = 'opened') THEN
    RAISE EXCEPTION 'reconciliation % has no opened event', NEW.id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

-- A finalised reconciliation's items are inserted in the transaction
-- that finalises it, and never afterwards.
CREATE FUNCTION finance_reconciliation_refuse_late_item() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM finance_reconciliation_reconciliation_events
    WHERE reconciliation_id = NEW.reconciliation_id AND action = 'finalised'
      AND pg_xact_status(
    ((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8
  ) = 'in progress'
  ) THEN
    RAISE EXCEPTION 'reconciliation % is not being finalised: an item cannot be added',
      NEW.reconciliation_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- Every table but the bank accounts is append-only, and none is ever truncated.

CREATE TRIGGER finance_reconciliation_bank_account_changes_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_bank_account_changes
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_statements_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_statements
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_statement_lines_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_statement_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_matches_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_matches
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_lines_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_match_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_entries_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_match_entries
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_events_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_match_events
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliations_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_reconciliations
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliation_events_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_reconciliation_events
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliation_items_append_only BEFORE UPDATE OR DELETE ON finance_reconciliation_reconciliation_items
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_bank_accounts_no_truncate BEFORE TRUNCATE ON finance_reconciliation_bank_accounts
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_bank_account_changes_no_truncate BEFORE TRUNCATE ON finance_reconciliation_bank_account_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_statements_no_truncate BEFORE TRUNCATE ON finance_reconciliation_statements
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_statement_lines_no_truncate BEFORE TRUNCATE ON finance_reconciliation_statement_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_matches_no_truncate BEFORE TRUNCATE ON finance_reconciliation_matches
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_lines_no_truncate BEFORE TRUNCATE ON finance_reconciliation_match_lines
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_entries_no_truncate BEFORE TRUNCATE ON finance_reconciliation_match_entries
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_match_events_no_truncate BEFORE TRUNCATE ON finance_reconciliation_match_events
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliations_no_truncate BEFORE TRUNCATE ON finance_reconciliation_reconciliations
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliation_events_no_truncate BEFORE TRUNCATE ON finance_reconciliation_reconciliation_events
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_reconciliation_items_no_truncate BEFORE TRUNCATE ON finance_reconciliation_reconciliation_items
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();

-- The database numbers and stamps each company's rows of these kinds.

CREATE TRIGGER finance_reconciliation_statements_number BEFORE INSERT ON finance_reconciliation_statements
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_number();

CREATE TRIGGER finance_reconciliation_matches_number BEFORE INSERT ON finance_reconciliation_matches
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_number();

CREATE TRIGGER finance_reconciliation_match_events_number BEFORE INSERT ON finance_reconciliation_match_events
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_number();

CREATE TRIGGER finance_reconciliation_reconciliations_number BEFORE INSERT ON finance_reconciliation_reconciliations
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_number();

CREATE TRIGGER finance_reconciliation_reconciliation_events_number BEFORE INSERT ON finance_reconciliation_reconciliation_events
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_number();

CREATE TRIGGER finance_reconciliation_statement_lines_open_parent BEFORE INSERT ON finance_reconciliation_statement_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_closed_part();

CREATE TRIGGER finance_reconciliation_match_lines_open_parent BEFORE INSERT ON finance_reconciliation_match_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_closed_part();

CREATE TRIGGER finance_reconciliation_match_entries_open_parent BEFORE INSERT ON finance_reconciliation_match_entries
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_closed_part();

CREATE CONSTRAINT TRIGGER finance_reconciliation_statements_complete AFTER INSERT ON finance_reconciliation_statements
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_statement();

CREATE CONSTRAINT TRIGGER finance_reconciliation_statement_lines_complete AFTER INSERT ON finance_reconciliation_statement_lines
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_statement();

CREATE CONSTRAINT TRIGGER finance_reconciliation_matches_balanced AFTER INSERT ON finance_reconciliation_matches
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_match();

CREATE CONSTRAINT TRIGGER finance_reconciliation_match_lines_balanced AFTER INSERT ON finance_reconciliation_match_lines
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_match();

CREATE CONSTRAINT TRIGGER finance_reconciliation_match_entries_balanced AFTER INSERT ON finance_reconciliation_match_entries
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_match();

CREATE TRIGGER finance_reconciliation_bank_accounts_kept
  BEFORE DELETE ON finance_reconciliation_bank_accounts FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();

CREATE TRIGGER finance_reconciliation_bank_accounts_identity
  BEFORE UPDATE ON finance_reconciliation_bank_accounts
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_keep_identity();

CREATE TRIGGER finance_reconciliation_bank_account_changes_stamp
  BEFORE INSERT ON finance_reconciliation_bank_account_changes
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_stamp_change();

CREATE TRIGGER finance_reconciliation_match_lines_free
  BEFORE INSERT ON finance_reconciliation_match_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_taken_line();

CREATE TRIGGER finance_reconciliation_match_entries_free
  BEFORE INSERT ON finance_reconciliation_match_entries
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_taken_entry();

CREATE TRIGGER finance_reconciliation_match_events_record
  BEFORE INSERT ON finance_reconciliation_match_events
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_record_match_event();

CREATE TRIGGER finance_reconciliation_reconciliations_open
  BEFORE INSERT ON finance_reconciliation_reconciliations
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_open_reconciliation();

CREATE CONSTRAINT TRIGGER finance_reconciliation_reconciliations_opened
  AFTER INSERT ON finance_reconciliation_reconciliations DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_reconciliation();

CREATE TRIGGER finance_reconciliation_reconciliation_events_record
  BEFORE INSERT ON finance_reconciliation_reconciliation_events
  FOR EACH ROW
  EXECUTE FUNCTION finance_reconciliation_record_reconciliation_event();

CREATE TRIGGER finance_reconciliation_reconciliation_items_finalising
  BEFORE INSERT ON finance_reconciliation_reconciliation_items
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_late_item();
