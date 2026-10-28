-- Each register and each source export remains immutable. Replacements switch
-- membership atomically; postings and historical report boundaries do not move.
ALTER TABLE finance_fixed_assets_opening_batches DROP CONSTRAINT finance_fixed_assets_opening_batches_company_id_key;
ALTER TABLE finance_fixed_assets_opening_batches ADD UNIQUE(company_id,source_reference);
ALTER TABLE finance_fixed_assets_opening_items DROP CONSTRAINT finance_fixed_assets_opening_items_company_id_source_id_key;
ALTER TABLE finance_fixed_assets_opening_items ADD UNIQUE(batch_id,source_id);
CREATE TABLE finance_fixed_assets_opening_stages (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 source_reference varchar(100) NOT NULL, part integer NOT NULL CHECK(part BETWEEN 1 AND 200),
 payload jsonb NOT NULL CHECK(octet_length(payload::text)<=262144),
 recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
 recorded_at timestamp(6) NOT NULL DEFAULT(clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(company_id,source_reference,part)
);
CREATE TABLE finance_fixed_assets_opening_decisions (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 batch_id bigint NOT NULL, replaces_batch_id bigint, reason text, effective_date date NOT NULL,
 review jsonb NOT NULL, approval jsonb NOT NULL,
 state text NOT NULL DEFAULT 'reconciled' CHECK(state='reconciled'),
 recorded_by_user_id bigint NOT NULL, recorded_by_impersonator_id bigint,
 recorded_at timestamp(6) NOT NULL DEFAULT(clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE(batch_id), UNIQUE(replaces_batch_id),
 CHECK(replaces_batch_id IS NULL OR (replaces_batch_id<batch_id AND length(btrim(reason)) BETWEEN 1 AND 500)),
 FOREIGN KEY(batch_id,company_id) REFERENCES finance_fixed_assets_opening_batches(id,company_id),
 FOREIGN KEY(replaces_batch_id,company_id) REFERENCES finance_fixed_assets_opening_batches(id,company_id)
);
CREATE INDEX finance_fixed_assets_opening_stages_scope ON finance_fixed_assets_opening_stages(tenant_id,company_id,id);
CREATE TRIGGER finance_fixed_assets_opening_stages_kept BEFORE UPDATE OR DELETE ON finance_fixed_assets_opening_stages FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_refuse();
CREATE TRIGGER finance_fixed_assets_opening_stages_retain BEFORE TRUNCATE ON finance_fixed_assets_opening_stages FOR EACH STATEMENT EXECUTE FUNCTION finance_fixed_assets_refuse();
CREATE INDEX finance_fixed_assets_opening_decisions_scope ON finance_fixed_assets_opening_decisions(tenant_id,company_id,id);
CREATE TRIGGER finance_fixed_assets_opening_decisions_kept BEFORE UPDATE OR DELETE ON finance_fixed_assets_opening_decisions FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_refuse();
CREATE TRIGGER finance_fixed_assets_opening_decisions_retain BEFORE TRUNCATE ON finance_fixed_assets_opening_decisions FOR EACH STATEMENT EXECUTE FUNCTION finance_fixed_assets_refuse();

ALTER TABLE finance_fixed_assets_opening_batches DROP CONSTRAINT finance_fixed_assets_opening_batches_source_csv_check;
ALTER TABLE finance_fixed_assets_opening_batches ADD CONSTRAINT finance_fixed_assets_opening_batches_source_csv_check CHECK(octet_length(source_csv)<=33554432);
