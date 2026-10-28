-- Conversion source records are permanent evidence. One complete register per company.
CREATE TABLE finance_receivables_opening_batches (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 source_reference varchar(100) NOT NULL CHECK (btrim(source_reference) <> ''),
 source_csv text NOT NULL CHECK (octet_length(source_csv)<=131072),
 currency_code varchar(3) NOT NULL, conversion_date date NOT NULL,
 entry_id bigint NOT NULL, entry_number bigint NOT NULL,
 series_id bigint NOT NULL,
 recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
 recorded_at timestamp(6) NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(company_id), UNIQUE(id,company_id),
 FOREIGN KEY(series_id,company_id) REFERENCES finance_receivables_series(id,company_id)
);
CREATE TABLE finance_receivables_opening_items (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 batch_id bigint NOT NULL, source_id varchar(200) NOT NULL,
 document_id bigint NOT NULL,
 source_facts jsonb NOT NULL,
 UNIQUE(company_id,source_id), UNIQUE(document_id),
 FOREIGN KEY(batch_id,company_id) REFERENCES finance_receivables_opening_batches(id,company_id),
 FOREIGN KEY(document_id,company_id) REFERENCES finance_receivables_documents(id,company_id)
);
CREATE INDEX finance_receivables_opening_batches_scope ON finance_receivables_opening_batches(tenant_id,company_id);
CREATE INDEX finance_receivables_opening_items_scope ON finance_receivables_opening_items(tenant_id,company_id);
CREATE TRIGGER finance_receivables_opening_batches_kept BEFORE UPDATE OR DELETE ON finance_receivables_opening_batches FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_opening_batches_retain BEFORE TRUNCATE ON finance_receivables_opening_batches FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_opening_items_kept BEFORE UPDATE OR DELETE ON finance_receivables_opening_items FOR EACH ROW EXECUTE FUNCTION finance_receivables_refuse_change();
CREATE TRIGGER finance_receivables_opening_items_retain BEFORE TRUNCATE ON finance_receivables_opening_items FOR EACH STATEMENT EXECUTE FUNCTION finance_receivables_refuse_change();

CREATE FUNCTION finance_receivables_opening_stamp() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC'; RETURN NEW; END $$;
CREATE TRIGGER finance_receivables_opening_stamp BEFORE INSERT ON finance_receivables_opening_batches FOR EACH ROW EXECUTE FUNCTION finance_receivables_opening_stamp();
