-- Workflow owns the approval round. Finance retains only durable references
-- to it; it does not copy approval state.
CREATE TABLE finance_ledger_journal_approvals (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    journal_id bigint NOT NULL,
    request_id bigint NOT NULL,
    FOREIGN KEY(journal_id,company_id) REFERENCES finance_ledger_journals(id,company_id),
    UNIQUE(journal_id), UNIQUE(tenant_id,request_id)
);
CREATE TRIGGER immutable_record BEFORE UPDATE OR DELETE ON finance_ledger_journal_approvals
    FOR EACH ROW EXECUTE FUNCTION finance_ledger_refuse_record_change();
CREATE TRIGGER immutable_table BEFORE TRUNCATE ON finance_ledger_journal_approvals
    FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_refuse_record_change();

-- The immutable decision and entry stay here. Workflow now proves who may
-- decide, records its exception once, and applies this effect in that same
-- audited transaction. The unique journal event remains the posting fence.
CREATE OR REPLACE FUNCTION finance_ledger_record_journal_event() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE journal finance_ledger_journals%ROWTYPE;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM finance_ledger_journal_approvals
        WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND journal_id=NEW.journal_id) THEN
        RAISE EXCEPTION 'journal % has no Workflow approval round; submit it through Workflow', NEW.journal_id
            USING ERRCODE='restrict_violation';
    END IF;
    SELECT * INTO journal FROM finance_ledger_journals
        WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND id=NEW.journal_id;
    IF NEW.event='withdrawn' AND FOUND AND NEW.actor_user_id<>journal.submitted_by_user_id THEN
        RAISE EXCEPTION 'journal % is withdrawn by someone other than whoever submitted it', NEW.journal_id
            USING ERRCODE='restrict_violation';
    END IF;
    NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
    RETURN NEW;
END; $$;
