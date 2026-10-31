-- Opening supplier credits are not new debit notes or purchases.
ALTER TABLE finance_payables_documents DROP CONSTRAINT finance_payables_documents_document_type;
ALTER TABLE finance_payables_documents ADD CONSTRAINT finance_payables_documents_document_type CHECK(document_type IN ('bill','debit_note','opening_credit'));
CREATE FUNCTION finance_payables_opening_credit_source() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.document_type='opening_credit' AND NOT EXISTS(SELECT 1 FROM finance_payables_opening_items i WHERE i.tenant_id=NEW.tenant_id AND i.company_id=NEW.company_id AND i.document_id=NEW.id AND i.batch_id=NEW.opening_batch_id AND i.source_facts->>'item_kind'='supplier_credit') THEN
 RAISE EXCEPTION 'opening credit requires its immutable source item' USING ERRCODE='check_violation'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER finance_payables_opening_credit_source AFTER INSERT OR UPDATE ON finance_payables_documents DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_payables_opening_credit_source();
