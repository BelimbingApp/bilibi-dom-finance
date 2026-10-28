-- finance/ledger 006: account groups. A group is a row of the chart that takes
-- no postings and stands for the total of everything under it; an account is
-- a row that takes postings and has no children. `parent_id` now always names
-- the enclosing group, so a chart is a tree whose leaves are accounts and whose
-- inner rows are groups.
--
-- Version 001 let any account parent another. That model is replaced, not
-- kept beside the new one: every row that already parents another becomes a
-- group, and one that cannot (it holds postings, or retained earnings) stops
-- this migration with the rows named so a disposable book is recreated rather
-- than reinterpreted. The facade and its callers change in the same release.

ALTER TABLE finance_ledger_accounts ADD COLUMN is_group boolean NOT NULL DEFAULT false;

DO $$
DECLARE
  offending text;
BEGIN
  SELECT string_agg(DISTINCT p.code, ', ' ORDER BY p.code) INTO offending
  FROM finance_ledger_accounts p
  WHERE EXISTS (SELECT 1 FROM finance_ledger_accounts c WHERE c.parent_id = p.id AND c.company_id = p.company_id)
    AND (p.retained_earnings
         OR EXISTS (SELECT 1 FROM finance_ledger_entry_lines l WHERE l.account_id = p.id AND l.company_id = p.company_id));
  IF offending IS NOT NULL THEN
    RAISE EXCEPTION 'account(s) % group other accounts but hold postings or retained earnings; recreate the book on the account group model', offending
      USING ERRCODE = 'restrict_violation';
  END IF;
END
$$;

UPDATE finance_ledger_accounts p SET is_group = true, active = true
WHERE EXISTS (SELECT 1 FROM finance_ledger_accounts c WHERE c.parent_id = p.id AND c.company_id = p.company_id);

-- A group is never deactivated and never designated for retained earnings: it
-- holds nothing of its own to retire or to close profit into.
ALTER TABLE finance_ledger_accounts
  ADD CONSTRAINT finance_ledger_accounts_group_active CHECK (NOT is_group OR active),
  ADD CONSTRAINT finance_ledger_accounts_group_not_retained CHECK (NOT is_group OR NOT retained_earnings);

-- A row is an account or a group for good (postings and children depend on
-- it), as it keeps its company, code and type.
CREATE OR REPLACE FUNCTION finance_ledger_accounts_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR NEW.code <> OLD.code OR NEW.account_type <> OLD.account_type
    OR NEW.is_group <> OLD.is_group THEN
    RAISE EXCEPTION 'a ledger account keeps its company, code, type and whether it is a group'
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

-- Only a group of the same type encloses a row.
CREATE FUNCTION finance_ledger_accounts_check_parent() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  parent finance_ledger_accounts%ROWTYPE;
BEGIN
  IF NEW.parent_id IS NULL THEN
    RETURN NEW;
  END IF;

  SELECT * INTO parent FROM finance_ledger_accounts
  WHERE id = NEW.parent_id AND company_id = NEW.company_id;

  IF NOT FOUND OR NOT parent.is_group THEN
    RAISE EXCEPTION 'a ledger account is grouped only under an account group'
      USING ERRCODE = 'restrict_violation';
  END IF;
  IF parent.account_type <> NEW.account_type THEN
    RAISE EXCEPTION 'a ledger account is grouped only under a group of its own type'
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_accounts_parent
  BEFORE INSERT OR UPDATE OF parent_id ON finance_ledger_accounts
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_accounts_check_parent();

-- A line posts to an account, never to a group.
CREATE FUNCTION finance_ledger_entry_lines_check_account() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM finance_ledger_accounts
             WHERE id = NEW.account_id AND company_id = NEW.company_id AND is_group) THEN
    RAISE EXCEPTION 'an account group takes no postings; post to an account under it'
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;
CREATE TRIGGER finance_ledger_entry_lines_account
  BEFORE INSERT ON finance_ledger_entry_lines
  FOR EACH ROW EXECUTE FUNCTION finance_ledger_entry_lines_check_account();
