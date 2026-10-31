-- Opening credits are retained sources, not newly issued taxable credit notes.
ALTER TABLE finance_receivables_documents DROP CONSTRAINT finance_receivables_documents_kind;
ALTER TABLE finance_receivables_documents ADD CONSTRAINT finance_receivables_documents_kind CHECK(kind IN ('invoice','credit_note','opening_credit'));
ALTER TABLE finance_receivables_documents DROP CONSTRAINT finance_receivables_documents_series_fkey;
ALTER TABLE finance_receivables_documents ADD CONSTRAINT finance_receivables_documents_series_fkey FOREIGN KEY(series_id,company_id) REFERENCES finance_receivables_series(id,company_id);
CREATE FUNCTION finance_receivables_document_series_kind() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM finance_receivables_series s WHERE s.id=NEW.series_id AND s.company_id=NEW.company_id AND s.tenant_id=NEW.tenant_id AND s.kind=CASE WHEN NEW.kind='opening_credit' THEN 'invoice' ELSE NEW.kind END) THEN
 RAISE EXCEPTION 'document series must match its kind' USING ERRCODE='check_violation'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER finance_receivables_document_series_kind BEFORE INSERT OR UPDATE ON finance_receivables_documents FOR EACH ROW EXECUTE FUNCTION finance_receivables_document_series_kind();
-- A credit may only commit when the complete reviewed opening register owns it.
CREATE FUNCTION finance_receivables_opening_credit_source() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.kind='opening_credit' AND NOT EXISTS(SELECT 1 FROM finance_receivables_opening_items i WHERE i.tenant_id=NEW.tenant_id AND i.company_id=NEW.company_id AND i.document_id=NEW.id AND i.source_facts->>'item_kind'='customer_credit') THEN
 RAISE EXCEPTION 'opening credit requires its immutable source item' USING ERRCODE='check_violation'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER finance_receivables_opening_credit_source AFTER INSERT OR UPDATE ON finance_receivables_documents DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_receivables_opening_credit_source();
