-- Opening items describe the bank side of an already posted opening entry.
-- They never post the balance a second time. Replacements retain the source
-- and items they correct; a bridge is assembled in its parent's transaction.
CREATE TABLE finance_reconciliation_opening_bridges (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bank_account_id bigint NOT NULL,
  source_reference varchar(100) NOT NULL CHECK (btrim(source_reference) <> ''),
  statement_source varchar(500) NOT NULL CHECK (btrim(statement_source) <> ''),
  source_csv text NOT NULL,
  review jsonb NOT NULL,
  conversion_date date NOT NULL,
  statement_balance numeric(20,4) NOT NULL,
  book_balance numeric(20,4) NOT NULL,
  opening_entry_id bigint NOT NULL,
  opening_entry_number bigint NOT NULL CHECK (opening_entry_number > 0),
  item_count integer NOT NULL CHECK (item_count BETWEEN 0 AND 500),
  replaces_bridge_id bigint UNIQUE,
  reason varchar(500),
  recorded_by_user_id bigint NOT NULL,
  recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC'),
  UNIQUE (id, company_id),
  UNIQUE (company_id, source_reference),
  FOREIGN KEY (bank_account_id, company_id)
    REFERENCES finance_reconciliation_bank_accounts (id, company_id) ON DELETE RESTRICT,
  FOREIGN KEY (replaces_bridge_id, company_id)
    REFERENCES finance_reconciliation_opening_bridges (id, company_id) ON DELETE RESTRICT,
  CHECK ((replaces_bridge_id IS NULL AND reason IS NULL) OR
         (replaces_bridge_id IS NOT NULL AND reason IS NOT NULL AND btrim(reason) <> ''))
);
CREATE UNIQUE INDEX finance_reconciliation_opening_root
  ON finance_reconciliation_opening_bridges (company_id, bank_account_id)
  WHERE replaces_bridge_id IS NULL;

CREATE TABLE finance_reconciliation_opening_items (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  bridge_id bigint NOT NULL,
  source_id varchar(200) NOT NULL CHECK (btrim(source_id) <> ''),
  item_date date NOT NULL,
  description varchar(500) NOT NULL CHECK (btrim(description) <> ''),
  amount numeric(20,4) NOT NULL CHECK (amount <> 0),
  UNIQUE (id, company_id),
  UNIQUE (bridge_id, source_id),
  FOREIGN KEY (bridge_id, company_id)
    REFERENCES finance_reconciliation_opening_bridges (id, company_id) ON DELETE RESTRICT
);

CREATE TABLE finance_reconciliation_match_opening_items (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  match_id bigint NOT NULL,
  opening_item_id bigint NOT NULL,
  UNIQUE (match_id, opening_item_id),
  FOREIGN KEY (match_id, company_id)
    REFERENCES finance_reconciliation_matches (id, company_id) ON DELETE RESTRICT,
  FOREIGN KEY (opening_item_id, company_id)
    REFERENCES finance_reconciliation_opening_items (id, company_id) ON DELETE RESTRICT
);
CREATE INDEX finance_reconciliation_match_opening_items_item
  ON finance_reconciliation_match_opening_items (opening_item_id);

ALTER TABLE finance_reconciliation_reconciliation_events ADD COLUMN through_opening_id bigint;
ALTER TABLE finance_reconciliation_reconciliation_events ADD CONSTRAINT finance_reconciliation_reconciliation_events_opening_pin
  CHECK ((action = 'finalised') = (through_opening_id IS NOT NULL));
ALTER TABLE finance_reconciliation_reconciliation_items ADD COLUMN opening_item_id bigint;
ALTER TABLE finance_reconciliation_reconciliation_items ADD CONSTRAINT finance_reconciliation_reconciliation_items_opening_fkey
  FOREIGN KEY (opening_item_id, company_id)
  REFERENCES finance_reconciliation_opening_items (id, company_id) ON DELETE RESTRICT;
ALTER TABLE finance_reconciliation_reconciliation_items DROP CONSTRAINT finance_reconciliation_reconciliation_items_kind;
ALTER TABLE finance_reconciliation_reconciliation_items DROP CONSTRAINT finance_reconciliation_reconciliation_items_subject;
ALTER TABLE finance_reconciliation_reconciliation_items ADD CONSTRAINT finance_reconciliation_reconciliation_items_kind
  CHECK (kind IN ('statement_line', 'entry', 'timing', 'opening_item'));
ALTER TABLE finance_reconciliation_reconciliation_items ADD CONSTRAINT finance_reconciliation_reconciliation_items_subject
  CHECK ((kind = 'statement_line') = (statement_line_id IS NOT NULL)
     AND (kind = 'entry') = (entry_id IS NOT NULL)
     AND (kind = 'timing') = (match_id IS NOT NULL)
     AND (kind = 'opening_item') = (opening_item_id IS NOT NULL));

CREATE TRIGGER finance_reconciliation_opening_bridges_kept BEFORE UPDATE OR DELETE
  ON finance_reconciliation_opening_bridges FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_opening_bridges_retain BEFORE TRUNCATE
  ON finance_reconciliation_opening_bridges FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_opening_items_kept BEFORE UPDATE OR DELETE
  ON finance_reconciliation_opening_items FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_opening_items_retain BEFORE TRUNCATE
  ON finance_reconciliation_opening_items FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_match_opening_items_kept BEFORE UPDATE OR DELETE
  ON finance_reconciliation_match_opening_items FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_match_opening_items_retain BEFORE TRUNCATE
  ON finance_reconciliation_match_opening_items FOR EACH STATEMENT EXECUTE FUNCTION finance_reconciliation_refuse_change();
CREATE TRIGGER finance_reconciliation_match_opening_items_open_parent BEFORE INSERT
  ON finance_reconciliation_match_opening_items FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_refuse_closed_part();

CREATE FUNCTION finance_reconciliation_opening_item_parent() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE open boolean;
BEGIN
  SELECT pg_xact_status(((pg_current_xact_id()::text::bigint >> 32 << 32) | xmin::text::bigint)::text::xid8) = 'in progress'
    INTO open FROM finance_reconciliation_opening_bridges WHERE id = NEW.bridge_id;
  IF NOT COALESCE(open, false) THEN
    RAISE EXCEPTION 'opening bridge % is already recorded', NEW.bridge_id USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_reconciliation_opening_item_parent BEFORE INSERT ON finance_reconciliation_opening_items
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_item_parent();

CREATE FUNCTION finance_reconciliation_opening_bridge_complete() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE n bigint; total numeric;
BEGIN
  SELECT count(*), COALESCE(sum(amount), 0) INTO n, total FROM finance_reconciliation_opening_items WHERE bridge_id = NEW.id;
  IF n <> NEW.item_count OR NEW.statement_balance - NEW.book_balance + total <> 0 OR
     EXISTS (SELECT 1 FROM finance_reconciliation_opening_items WHERE bridge_id = NEW.id AND item_date > NEW.conversion_date) THEN
    RAISE EXCEPTION 'opening bridge % does not reconcile', NEW.id USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER finance_reconciliation_opening_bridge_complete AFTER INSERT ON finance_reconciliation_opening_bridges
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_bridge_complete();

CREATE FUNCTION finance_reconciliation_opening_bridge_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM finance_reconciliation_reconciliations r
    JOIN finance_reconciliation_reconciliation_events e ON e.reconciliation_id = r.id AND e.action = 'finalised'
    WHERE r.tenant_id = NEW.tenant_id AND r.company_id = NEW.company_id AND r.bank_account_id = NEW.bank_account_id) THEN
    RAISE EXCEPTION 'a finalised reconciliation prevents changing the opening bridge' USING ERRCODE = 'restrict_violation';
  END IF;
  IF NEW.replaces_bridge_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM finance_reconciliation_opening_bridges b
    WHERE b.id = NEW.replaces_bridge_id AND b.tenant_id = NEW.tenant_id AND b.company_id = NEW.company_id
      AND b.bank_account_id = NEW.bank_account_id AND b.opening_entry_id = NEW.opening_entry_id
      AND b.conversion_date = NEW.conversion_date AND b.book_balance = NEW.book_balance
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges r WHERE r.replaces_bridge_id = b.id)
  ) THEN
    RAISE EXCEPTION 'replacement must name the current bridge of the same account and opening entry' USING ERRCODE = 'check_violation';
  END IF;
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_matches m
    JOIN finance_reconciliation_match_events e ON e.match_id = m.id AND e.action = 'matched'
    WHERE m.tenant_id = NEW.tenant_id AND m.company_id = NEW.company_id AND m.bank_account_id = NEW.bank_account_id
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events u WHERE u.match_id = m.id AND u.action = 'unmatched')
  ) THEN
    RAISE EXCEPTION 'unmatch the account before recording the opening bridge' USING ERRCODE = 'restrict_violation';
  END IF;
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_statement_lines s
    WHERE s.tenant_id = NEW.tenant_id AND s.company_id = NEW.company_id
      AND s.bank_account_id = NEW.bank_account_id AND s.line_date < NEW.conversion_date
  ) THEN
    RAISE EXCEPTION 'opening conversion follows an earlier statement line' USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_reconciliation_opening_bridge_guard BEFORE INSERT ON finance_reconciliation_opening_bridges
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_bridge_guard();

CREATE FUNCTION finance_reconciliation_opening_match_guard() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM finance_reconciliation_matches m
    JOIN finance_reconciliation_opening_items i ON i.id = NEW.opening_item_id AND i.company_id = m.company_id AND i.tenant_id = m.tenant_id
    JOIN finance_reconciliation_opening_bridges b ON b.id = i.bridge_id AND b.company_id = i.company_id
    WHERE m.id = NEW.match_id AND m.tenant_id = NEW.tenant_id AND m.company_id = NEW.company_id
      AND m.bank_account_id = b.bank_account_id
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges r WHERE r.replaces_bridge_id = b.id)
  ) THEN
    RAISE EXCEPTION 'opening item is not current on this bank account' USING ERRCODE = 'check_violation';
  END IF;
  IF EXISTS (
    SELECT 1 FROM finance_reconciliation_match_opening_items i
    WHERE i.opening_item_id = NEW.opening_item_id AND i.match_id <> NEW.match_id
      AND EXISTS (SELECT 1 FROM finance_reconciliation_match_events e WHERE e.match_id = i.match_id AND e.action = 'matched')
      AND NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events e WHERE e.match_id = i.match_id AND e.action = 'unmatched')
  ) THEN
    RAISE EXCEPTION 'opening item is already matched' USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_reconciliation_opening_match_guard BEFORE INSERT ON finance_reconciliation_match_opening_items
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_match_guard();

-- The same match and unmatched-event controls apply to opening items; the
-- opening item contributes to the book side without creating a ledger entry.
CREATE OR REPLACE FUNCTION finance_reconciliation_check_match() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE checked_id bigint; matched record; statement_side numeric; ledger_side numeric; opening_side numeric;
BEGIN
  IF TG_TABLE_NAME = 'finance_reconciliation_matches' THEN checked_id := NEW.id;
  ELSE checked_id := NEW.match_id; END IF;
  SELECT * INTO matched FROM finance_reconciliation_matches WHERE id = checked_id;
  SELECT sum(s.amount) INTO statement_side FROM finance_reconciliation_match_lines l
    JOIN finance_reconciliation_statement_lines s ON s.id = l.statement_line_id
    WHERE l.match_id = checked_id AND s.bank_account_id = matched.bank_account_id;
  SELECT sum(amount) INTO ledger_side FROM finance_reconciliation_match_entries
    WHERE match_id = checked_id AND bank_account_id = matched.bank_account_id;
  SELECT sum(i.amount) INTO opening_side FROM finance_reconciliation_match_opening_items x
    JOIN finance_reconciliation_opening_items i ON i.id = x.opening_item_id
    JOIN finance_reconciliation_opening_bridges b ON b.id = i.bridge_id
    WHERE x.match_id = checked_id AND b.bank_account_id = matched.bank_account_id;
  IF statement_side IS NULL OR (ledger_side IS NULL AND opening_side IS NULL) OR
     statement_side <> COALESCE(ledger_side, 0) + COALESCE(opening_side, 0) OR
     NOT EXISTS (SELECT 1 FROM finance_reconciliation_match_events WHERE match_id = checked_id AND action = 'matched') THEN
    RAISE EXCEPTION 'match % must have equal statement and book sides and a matched event', checked_id USING ERRCODE = 'check_violation';
  END IF;
  RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER finance_reconciliation_match_opening_items_balanced AFTER INSERT ON finance_reconciliation_match_opening_items
  DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_check_match();

CREATE FUNCTION finance_reconciliation_opening_entry_not_matchable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges b WHERE b.tenant_id=NEW.tenant_id AND b.company_id=NEW.company_id AND b.bank_account_id=NEW.bank_account_id AND b.opening_entry_id=NEW.entry_id) THEN
    RAISE EXCEPTION 'opening entry is represented by outstanding opening items' USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_reconciliation_opening_entry_not_matchable BEFORE INSERT ON finance_reconciliation_match_entries
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_entry_not_matchable();

CREATE FUNCTION finance_reconciliation_opening_statement_cutoff() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM finance_reconciliation_opening_bridges b WHERE b.tenant_id=NEW.tenant_id AND b.company_id=NEW.company_id AND b.bank_account_id=NEW.bank_account_id AND NEW.line_date < b.conversion_date) THEN
    RAISE EXCEPTION 'statement line is before opening conversion' USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER finance_reconciliation_opening_statement_cutoff BEFORE INSERT ON finance_reconciliation_statement_lines
  FOR EACH ROW EXECUTE FUNCTION finance_reconciliation_opening_statement_cutoff();
