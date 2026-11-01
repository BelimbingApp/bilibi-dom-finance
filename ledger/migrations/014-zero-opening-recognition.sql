-- A reviewed zero control still depends on the approved opening entry. An
-- absent line means zero; recognition retains the same permanent reversal fence.
CREATE FUNCTION finance_ledger_opening_recognition_balance() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM finance_ledger_accounts a WHERE a.id=NEW.account_id AND a.tenant_id=NEW.tenant_id AND a.company_id=NEW.company_id AND NOT a.is_group AND a.account_type IN ('asset','liability'))
 OR NOT EXISTS(SELECT 1 FROM finance_ledger_opening_balances o JOIN finance_ledger_journal_events j ON j.tenant_id=o.tenant_id AND j.company_id=o.company_id AND j.journal_id=o.journal_id AND j.event='approved' WHERE o.tenant_id=NEW.tenant_id AND o.company_id=NEW.company_id AND j.entry_id=NEW.entry_id)
 OR EXISTS(SELECT 1 FROM finance_ledger_entries e WHERE e.tenant_id=NEW.tenant_id AND e.company_id=NEW.company_id AND e.reverses_entry_id=NEW.entry_id)
 OR NEW.balance<>COALESCE((SELECT sum(l.debit-l.credit) FROM finance_ledger_entry_lines l WHERE l.tenant_id=NEW.tenant_id AND l.company_id=NEW.company_id AND l.entry_id=NEW.entry_id AND l.account_id=NEW.account_id),0)
 THEN RAISE EXCEPTION 'recognition must match an eligible approved opening control, including a reviewed zero' USING ERRCODE='check_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_ledger_opening_recognition_balance BEFORE INSERT ON finance_ledger_opening_recognitions FOR EACH ROW EXECUTE FUNCTION finance_ledger_opening_recognition_balance();
