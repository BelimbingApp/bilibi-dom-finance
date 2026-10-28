-- Each register and each source export remains immutable. Replacements switch
-- membership atomically; postings and historical report boundaries do not move.
ALTER TABLE finance_payables_opening_batches DROP CONSTRAINT finance_payables_opening_batches_company_id_key;
ALTER TABLE finance_payables_opening_batches ADD UNIQUE(company_id,source_reference);
ALTER TABLE finance_payables_opening_items DROP CONSTRAINT finance_payables_opening_items_company_id_source_id_key;
ALTER TABLE finance_payables_opening_items ADD UNIQUE(batch_id,source_id);
CREATE TABLE finance_payables_opening_stages (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 source_reference varchar(100) NOT NULL, part integer NOT NULL CHECK(part BETWEEN 1 AND 200),
 payload jsonb NOT NULL CHECK(octet_length(payload::text)<=262144),
 recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
 recorded_at timestamp(6) NOT NULL DEFAULT(clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(company_id,source_reference,part)
);
CREATE TABLE finance_payables_opening_decisions (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 batch_id bigint NOT NULL, replaces_batch_id bigint, reason text, effective_date date NOT NULL,
 review jsonb NOT NULL, approval jsonb NOT NULL,
 state text NOT NULL DEFAULT 'reconciled' CHECK(state='reconciled'),
 recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
 recorded_at timestamp(6) NOT NULL DEFAULT(clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(batch_id), UNIQUE(replaces_batch_id),
 CHECK(replaces_batch_id IS NULL OR (replaces_batch_id<batch_id AND length(btrim(reason)) BETWEEN 1 AND 500)),
 FOREIGN KEY(batch_id,company_id) REFERENCES finance_payables_opening_batches(id,company_id),
 FOREIGN KEY(replaces_batch_id,company_id) REFERENCES finance_payables_opening_batches(id,company_id)
);
CREATE INDEX finance_payables_opening_stages_scope ON finance_payables_opening_stages(tenant_id,company_id,id);
CREATE TRIGGER finance_payables_opening_stages_kept BEFORE UPDATE OR DELETE ON finance_payables_opening_stages FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();
CREATE TRIGGER finance_payables_opening_stages_retain BEFORE TRUNCATE ON finance_payables_opening_stages FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();
CREATE INDEX finance_payables_opening_decisions_scope ON finance_payables_opening_decisions(tenant_id,company_id,id);
CREATE TRIGGER finance_payables_opening_decisions_kept BEFORE UPDATE OR DELETE ON finance_payables_opening_decisions FOR EACH ROW EXECUTE FUNCTION finance_payables_refuse_change();
CREATE TRIGGER finance_payables_opening_decisions_retain BEFORE TRUNCATE ON finance_payables_opening_decisions FOR EACH STATEMENT EXECUTE FUNCTION finance_payables_refuse_change();

-- A corrected opening bill is a new immutable snapshot of the same source
-- document. Ordinary bill duplicates retain their original unique fence.
ALTER TABLE finance_payables_documents ADD COLUMN opening_batch_id bigint;
ALTER TABLE finance_payables_documents ADD FOREIGN KEY(opening_batch_id,company_id) REFERENCES finance_payables_opening_batches(id,company_id);
DROP INDEX finance_payables_documents_party_document_unique;
CREATE UNIQUE INDEX finance_payables_documents_party_document_unique ON finance_payables_documents(company_id,party_key,party_document_number) WHERE document_type='bill' AND status<>'voided' AND opening_batch_id IS NULL;
CREATE UNIQUE INDEX finance_payables_documents_opening_source_unique ON finance_payables_documents(company_id,opening_batch_id,party_key,party_document_number) WHERE opening_batch_id IS NOT NULL;

ALTER TABLE finance_payables_opening_batches DROP CONSTRAINT finance_payables_opening_batches_source_csv_check;
ALTER TABLE finance_payables_opening_batches ADD CONSTRAINT finance_payables_opening_batches_source_csv_check CHECK(octet_length(source_csv)<=33554432);
-- Ordinary bills must also respect the current opening register's source key.
CREATE FUNCTION finance_payables_opening_bill_duplicate() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.document_type='bill' AND NEW.status<>'voided' AND NEW.opening_batch_id IS NULL AND EXISTS(
  SELECT 1 FROM finance_payables_documents d
  WHERE d.tenant_id=NEW.tenant_id AND d.company_id=NEW.company_id AND d.id<>NEW.id
   AND d.party_key=NEW.party_key AND d.party_document_number=NEW.party_document_number
   AND d.opening_batch_id IS NOT NULL AND d.status<>'voided'
   AND NOT EXISTS(SELECT 1 FROM finance_payables_opening_decisions x WHERE x.tenant_id=NEW.tenant_id AND x.company_id=NEW.company_id AND x.replaces_batch_id=d.opening_batch_id)
 ) THEN RAISE EXCEPTION 'supplier document already belongs to the current opening register' USING ERRCODE='unique_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_payables_opening_bill_duplicate BEFORE INSERT OR UPDATE ON finance_payables_documents FOR EACH ROW EXECUTE FUNCTION finance_payables_opening_bill_duplicate();
