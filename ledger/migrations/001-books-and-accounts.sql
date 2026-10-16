-- finance/ledger 001: each company's book, its chart of accounts, and the
-- controlled-bookkeeping setting a new book opens with. The end state of the
-- Bilimbi ledger's schema for these three tables (b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9); entries, periods, journals and
-- the other controls follow in later versions.
--
-- Rows of one company reference each other through composite (id, company_id)
-- foreign keys, so an account cannot be grouped under another company's. A
-- book and a bookkeeping change are append-only: a trigger refuses UPDATE,
-- DELETE and TRUNCATE. An account is never deleted and keeps its company,
-- code and type; its name, parent, active flag and retained-earnings
-- designation may change. The database numbers and stamps each company's
-- bookkeeping changes and refuses one that changes nothing.

CREATE TABLE finance_ledger_books (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  currency_code varchar(3) NOT NULL,
  minor_units smallint NOT NULL,
  inserted_at timestamp(0) NOT NULL,
  CONSTRAINT finance_ledger_books_currency_code CHECK (currency_code ~ '^[A-Z]{3}$'),
  CONSTRAINT finance_ledger_books_minor_units CHECK (minor_units BETWEEN 0 AND 4)
);
CREATE UNIQUE INDEX finance_ledger_books_company_unique ON finance_ledger_books (company_id);
CREATE UNIQUE INDEX finance_ledger_books_company_currency_unique ON finance_ledger_books (company_id, currency_code);
CREATE INDEX finance_ledger_books_tenant_id_company_id_index ON finance_ledger_books (tenant_id, company_id);

CREATE TABLE finance_ledger_accounts (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  code varchar(50) NOT NULL,
  name varchar(200) NOT NULL,
  account_type varchar(16) NOT NULL,
  parent_id bigint,
  active boolean NOT NULL DEFAULT true,
  retained_earnings boolean NOT NULL DEFAULT false,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_ledger_accounts_account_type
    CHECK (account_type IN ('asset', 'liability', 'equity', 'income', 'expense')),
  CONSTRAINT finance_ledger_accounts_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id),
  CONSTRAINT finance_ledger_accounts_retained_earnings_equity
    CHECK (NOT retained_earnings OR account_type = 'equity')
);
CREATE UNIQUE INDEX finance_ledger_accounts_id_company_unique ON finance_ledger_accounts (id, company_id);
-- A self-reference needs the composite key above before it can exist.
ALTER TABLE finance_ledger_accounts ADD CONSTRAINT finance_ledger_accounts_parent_fkey
  FOREIGN KEY (parent_id, company_id) REFERENCES finance_ledger_accounts (id, company_id)
  ON DELETE RESTRICT;
CREATE UNIQUE INDEX finance_ledger_accounts_company_code_unique ON finance_ledger_accounts (company_id, code);
CREATE INDEX finance_ledger_accounts_tenant_id_company_id_index ON finance_ledger_accounts (tenant_id, company_id);
-- The year-end close moves profit into the one equity account a company designates.
CREATE UNIQUE INDEX finance_ledger_accounts_one_retained_earnings ON finance_ledger_accounts (company_id)
  WHERE retained_earnings;

CREATE FUNCTION finance_ledger_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'the finance ledger keeps % rows: % is refused; post a reversing entry',
    TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE TRIGGER finance_ledger_books_append_only
  BEFORE UPDATE OR DELETE ON finance_ledger_books
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_accounts_kept
  BEFORE DELETE ON finance_ledger_accounts
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_books_no_truncate
  BEFORE TRUNCATE ON finance_ledger_books
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_change();
CREATE TRIGGER finance_ledger_accounts_no_truncate
  BEFORE TRUNCATE ON finance_ledger_accounts
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_change();

CREATE FUNCTION finance_ledger_accounts_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR NEW.code <> OLD.code OR NEW.account_type <> OLD.account_type THEN
    RAISE EXCEPTION 'a ledger account keeps its company, code and type'
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE TRIGGER finance_ledger_accounts_identity
  BEFORE UPDATE ON finance_ledger_accounts
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_accounts_keep_identity();

-- Controlled bookkeeping: the setting is never a column but the latest of the
-- company's append-only changes, each with the actor, any impersonator, a
-- reason (required to turn it off) and the database's time. A company with
-- no change is not controlled; opening a book records the change that turns
-- it on.
CREATE TABLE finance_ledger_bookkeeping_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  change_number bigint NOT NULL,
  controlled boolean NOT NULL,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_ledger_bookkeeping_changes_change_number CHECK (change_number > 0),
  CONSTRAINT finance_ledger_bookkeeping_changes_reason
    CHECK (controlled OR (reason IS NOT NULL AND btrim(reason) <> ''))
);
CREATE UNIQUE INDEX finance_ledger_bookkeeping_changes_company_number_unique
  ON finance_ledger_bookkeeping_changes (company_id, change_number);
CREATE INDEX finance_ledger_bookkeeping_changes_tenant_company_index
  ON finance_ledger_bookkeeping_changes (tenant_id, company_id);

CREATE FUNCTION finance_ledger_refuse_record_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'the finance ledger keeps % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

-- The latest change decides; with none, a company is not controlled. A
-- change must change the setting.
CREATE FUNCTION finance_ledger_record_bookkeeping_change() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  current boolean;
BEGIN
  SELECT controlled INTO current FROM finance_ledger_bookkeeping_changes
  WHERE company_id = NEW.company_id ORDER BY change_number DESC LIMIT 1;

  IF COALESCE(current, false) = NEW.controlled THEN
    RAISE EXCEPTION 'controlled bookkeeping of company % is already %', NEW.company_id,
      CASE WHEN NEW.controlled THEN 'on' ELSE 'off' END
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.change_number := COALESCE(
    (SELECT max(change_number) FROM finance_ledger_bookkeeping_changes
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_ledger_company_controlled(target bigint) RETURNS boolean
LANGUAGE plpgsql STABLE AS $$
DECLARE
  latest boolean;
BEGIN
  SELECT controlled INTO latest FROM finance_ledger_bookkeeping_changes
  WHERE company_id = target ORDER BY change_number DESC LIMIT 1;

  RETURN COALESCE(latest, false);
END
$$;

CREATE TRIGGER finance_ledger_bookkeeping_changes_append_only
  BEFORE UPDATE OR DELETE ON finance_ledger_bookkeeping_changes
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_bookkeeping_changes_no_truncate
  BEFORE TRUNCATE ON finance_ledger_bookkeeping_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER finance_ledger_bookkeeping_changes_record
  BEFORE INSERT ON finance_ledger_bookkeeping_changes
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_record_bookkeeping_change();
