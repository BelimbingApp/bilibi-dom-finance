-- Settlement numbers are assigned from the committed company stream while
-- holding the company write lock. Neither an abort nor an entryless action
-- consumes a ledger entry number.
ALTER TABLE finance_receivables_opening_items ADD UNIQUE(id,company_id);
CREATE TABLE finance_receivables_settlements (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 settlement_number bigint NOT NULL CHECK(settlement_number>0),
 kind text NOT NULL CHECK(kind IN ('apply','refund','reverse')),
 credit_item_id bigint NOT NULL, credit_document_id bigint NOT NULL,
 target_document_id bigint, amount numeric(20,4) NOT NULL CHECK(amount>0),
 effective_date date NOT NULL, cash_account_id bigint,
 reference varchar(100) NOT NULL CHECK(length(btrim(reference))>0),
 reverses_settlement_id bigint, entry_id bigint, entry_number bigint,
 review jsonb NOT NULL, actor_user_id bigint NOT NULL, actor_impersonator_id bigint,
 audit_trace_id text NOT NULL, audit_action text NOT NULL,
 recorded_at timestamp(6) NOT NULL DEFAULT(clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(company_id,settlement_number), UNIQUE(id,company_id),
 UNIQUE(company_id,reference), UNIQUE(reverses_settlement_id),
 FOREIGN KEY(credit_item_id,company_id) REFERENCES finance_receivables_opening_items(id,company_id),
 FOREIGN KEY(credit_document_id,company_id) REFERENCES finance_receivables_documents(id,company_id),
 FOREIGN KEY(target_document_id,company_id) REFERENCES finance_receivables_documents(id,company_id),
 FOREIGN KEY(reverses_settlement_id,company_id) REFERENCES finance_receivables_settlements(id,company_id),
 FOREIGN KEY(entry_id,company_id) REFERENCES finance_ledger_entries(id,company_id),
 FOREIGN KEY(cash_account_id,company_id) REFERENCES finance_ledger_accounts(id,company_id),
 CHECK((entry_id IS NULL)=(entry_number IS NULL)),
 CHECK((kind='reverse')=(reverses_settlement_id IS NOT NULL)),
 CHECK(kind<>'apply' OR (target_document_id IS NOT NULL AND cash_account_id IS NULL)),
 CHECK(kind<>'refund' OR (target_document_id IS NULL AND cash_account_id IS NOT NULL AND entry_id IS NOT NULL)),
 CHECK(credit_document_id IS DISTINCT FROM target_document_id)
);
CREATE INDEX finance_receivables_settlements_credit ON finance_receivables_settlements(tenant_id,company_id,credit_document_id,effective_date,settlement_number);
CREATE INDEX finance_receivables_settlements_target ON finance_receivables_settlements(tenant_id,company_id,target_document_id,effective_date,settlement_number);
CREATE TRIGGER finance_receivables_settlements_kept BEFORE UPDATE OR DELETE ON finance_receivables_settlements FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_settlements_retain BEFORE TRUNCATE ON finance_receivables_settlements FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE FUNCTION finance_receivables_settlement_shape() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 -- This lock is also taken by Ledger::write; serialize even an accidental
 -- direct insert before checking the stream's next number.
 PERFORM id FROM companies WHERE tenant_id=NEW.tenant_id AND id=NEW.company_id FOR UPDATE;
 IF NEW.settlement_number<>(SELECT COALESCE(max(settlement_number),0)+1 FROM finance_receivables_settlements WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id) THEN
 RAISE EXCEPTION 'settlement number must be next for this company' USING ERRCODE='check_violation'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance_receivables_opening_items i WHERE i.tenant_id=NEW.tenant_id AND i.company_id=NEW.company_id AND i.id=NEW.credit_item_id AND i.document_id=NEW.credit_document_id AND i.source_facts->>'item_kind'='customer_credit') THEN
 RAISE EXCEPTION 'settlement requires an opening credit source' USING ERRCODE='check_violation'; END IF;
 IF NEW.kind='reverse' AND NOT EXISTS(SELECT 1 FROM finance_receivables_settlements e WHERE e.tenant_id=NEW.tenant_id AND e.company_id=NEW.company_id AND e.id=NEW.reverses_settlement_id AND e.kind<>'reverse' AND e.settlement_number<NEW.settlement_number AND e.credit_item_id=NEW.credit_item_id AND e.credit_document_id=NEW.credit_document_id AND e.target_document_id IS NOT DISTINCT FROM NEW.target_document_id AND e.cash_account_id IS NOT DISTINCT FROM NEW.cash_account_id AND e.amount=NEW.amount AND e.effective_date<=NEW.effective_date AND (e.entry_id IS NULL)=(NEW.entry_id IS NULL)) THEN
 RAISE EXCEPTION 'reversal must restore its original settlement' USING ERRCODE='check_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_receivables_settlement_shape BEFORE INSERT ON finance_receivables_settlements FOR EACH ROW EXECUTE FUNCTION finance_receivables_settlement_shape();
CREATE FUNCTION finance_receivables_residual(t bigint,c bigint,document bigint,day date,posting bigint,settlement bigint) RETURNS numeric LANGUAGE sql STABLE AS $$
 SELECT d.total
 - COALESCE((SELECT sum(n.total) FROM finance_receivables_documents n WHERE n.tenant_id=t AND n.company_id=c AND n.credited_document_id=d.id AND n.status='issued' AND n.issue_date<=day AND n.ledger_entry_number<=posting),0)
 - COALESCE((SELECT sum(a.amount) FROM finance_receivables_allocations a JOIN finance_receivables_payments p ON p.id=a.payment_id AND p.tenant_id=t AND p.company_id=c WHERE a.tenant_id=t AND a.company_id=c AND a.document_id=d.id AND p.payment_date<=day AND p.ledger_entry_number<=posting AND NOT EXISTS(SELECT 1 FROM finance_receivables_payment_reversals x WHERE x.tenant_id=t AND x.company_id=c AND x.payment_id=p.id AND x.reversal_date<=day AND x.ledger_entry_number<=posting)),0)
 - COALESCE((SELECT sum(w.amount) FROM finance_receivables_write_offs w WHERE w.tenant_id=t AND w.company_id=c AND w.document_id=d.id AND EXISTS(SELECT 1 FROM finance_receivables_write_off_events e WHERE e.tenant_id=t AND e.company_id=c AND e.write_off_id=w.id AND e.event='approved' AND e.effective_date<=day AND e.ledger_entry_number<=posting) AND NOT EXISTS(SELECT 1 FROM finance_receivables_write_off_events e WHERE e.tenant_id=t AND e.company_id=c AND e.write_off_id=w.id AND e.event='reversed' AND e.effective_date<=day AND e.ledger_entry_number<=posting)),0)
 - COALESCE((SELECT sum(CASE WHEN e.kind='reverse' THEN -e.amount ELSE e.amount END) FROM finance_receivables_settlements e WHERE e.tenant_id=t AND e.company_id=c AND (e.credit_document_id=d.id OR e.target_document_id=d.id) AND e.effective_date<=day AND e.settlement_number<=settlement AND (e.entry_number IS NULL OR e.entry_number<=posting)),0)
 FROM finance_receivables_documents d WHERE d.tenant_id=t AND d.company_id=c AND d.id=document
$$;
CREATE OR REPLACE FUNCTION finance_receivables_outstanding(invoice_id bigint) RETURNS numeric LANGUAGE sql STABLE AS $$
 SELECT finance_receivables_residual(d.tenant_id,d.company_id,d.id,'infinity'::date,9223372036854775807,9223372036854775807) FROM finance_receivables_documents d WHERE d.id=invoice_id
$$;
