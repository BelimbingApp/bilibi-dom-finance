-- Invoice-linked reviewed journals. No statutory tax eligibility is inferred.
CREATE TABLE finance_receivables_write_offs (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 document_id bigint NOT NULL, entry_date date NOT NULL,
 amount numeric(20,4) NOT NULL CHECK (amount > 0),
 offset_account_id bigint NOT NULL, reason varchar(500) NOT NULL CHECK (length(trim(reason)) > 0),
 evidence varchar(500) NOT NULL CHECK (length(trim(evidence)) > 0),
 submitted_by_user_id bigint NOT NULL, submitted_by_impersonator_id bigint,
 submitted_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 UNIQUE (id, tenant_id, company_id),
 FOREIGN KEY (document_id, company_id) REFERENCES finance_receivables_documents(id, company_id)
);
CREATE INDEX finance_receivables_write_offs_document ON finance_receivables_write_offs(tenant_id, company_id, document_id);
CREATE TABLE finance_receivables_write_off_events (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 write_off_id bigint NOT NULL, event varchar(16) NOT NULL CHECK (event IN ('approved','rejected','reversed')),
 effective_date date NOT NULL, reason varchar(500) NOT NULL CHECK (length(trim(reason)) > 0),
 ledger_entry_id bigint, ledger_entry_number bigint,
 actor_user_id bigint NOT NULL, actor_impersonator_id bigint,
 occurred_at timestamp(6) NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
 FOREIGN KEY (write_off_id, tenant_id, company_id) REFERENCES finance_receivables_write_offs(id, tenant_id, company_id),
 CHECK ((event = 'rejected' AND ledger_entry_id IS NULL AND ledger_entry_number IS NULL)
     OR (event <> 'rejected' AND ledger_entry_id IS NOT NULL AND ledger_entry_number > 0)),
 UNIQUE (write_off_id, event)
);
CREATE INDEX finance_receivables_write_off_events_company ON finance_receivables_write_off_events(tenant_id, company_id, ledger_entry_number);
CREATE UNIQUE INDEX finance_receivables_write_off_decision ON finance_receivables_write_off_events(write_off_id) WHERE event IN ('approved','rejected');
CREATE FUNCTION finance_receivables_guard_write_off() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE w finance_receivables_write_offs; d finance_receivables_documents;
BEGIN
 IF TG_TABLE_NAME = 'finance_receivables_write_offs' THEN
  SELECT * INTO STRICT d FROM finance_receivables_documents WHERE id=NEW.document_id AND tenant_id=NEW.tenant_id AND company_id=NEW.company_id;
  IF d.kind <> 'invoice' OR d.status <> 'issued' OR d.due_date >= NEW.entry_date OR d.due_date IS NULL
     OR finance_receivables_outstanding(d.id) < NEW.amount THEN
   RAISE EXCEPTION 'write-off requires an overdue invoice with enough outstanding';
  END IF;
 ELSE
  SELECT * INTO STRICT w FROM finance_receivables_write_offs WHERE id=NEW.write_off_id AND tenant_id=NEW.tenant_id AND company_id=NEW.company_id;
  IF NEW.event='approved' AND (NEW.effective_date <> w.entry_date OR finance_receivables_outstanding(w.document_id) < w.amount) THEN
   RAISE EXCEPTION 'write-off exceeds outstanding or changes its date';
  END IF;
  IF NEW.event='reversed' AND NOT EXISTS (SELECT 1 FROM finance_receivables_write_off_events WHERE write_off_id=w.id AND event='approved' AND effective_date<=NEW.effective_date) THEN
   RAISE EXCEPTION 'only an approved write-off may be reversed on or after its date';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_receivables_write_off_insert BEFORE INSERT ON finance_receivables_write_offs FOR EACH ROW EXECUTE FUNCTION finance_receivables_guard_write_off();
CREATE TRIGGER finance_receivables_write_off_event_insert BEFORE INSERT ON finance_receivables_write_off_events FOR EACH ROW EXECUTE FUNCTION finance_receivables_guard_write_off();
CREATE TRIGGER finance_receivables_write_off_immutable BEFORE UPDATE OR DELETE ON finance_receivables_write_offs FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_write_off_event_immutable BEFORE UPDATE OR DELETE ON finance_receivables_write_off_events FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();

-- Settlement capacity includes approved write-offs; tax schedules keep their
-- own gross unpaid calculation because a write-off is neither cash nor credit.
CREATE OR REPLACE FUNCTION finance_receivables_outstanding(invoice_id bigint) RETURNS numeric
LANGUAGE sql STABLE AS $$
 SELECT d.total
 - COALESCE((SELECT sum(c.total) FROM finance_receivables_documents c WHERE c.credited_document_id=d.id AND c.status='issued'),0)
 - COALESCE((SELECT sum(a.amount) FROM finance_receivables_allocations a WHERE a.document_id=d.id AND NOT EXISTS (SELECT 1 FROM finance_receivables_payment_reversals r WHERE r.payment_id=a.payment_id)),0)
 - COALESCE((SELECT sum(w.amount) FROM finance_receivables_write_offs w WHERE w.document_id=d.id AND EXISTS (SELECT 1 FROM finance_receivables_write_off_events e WHERE e.write_off_id=w.id AND e.event='approved') AND NOT EXISTS (SELECT 1 FROM finance_receivables_write_off_events e WHERE e.write_off_id=w.id AND e.event='reversed')),0)
 FROM finance_receivables_documents d WHERE d.id=invoice_id
$$;
