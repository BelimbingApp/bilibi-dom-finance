-- Published chart snapshots are tenant-shared company data. No balances,
-- entries, currency, report layout or subledger configuration are copied.
CREATE TABLE finance_ledger_chart_templates (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  key varchar(100) NOT NULL,
  template jsonb NOT NULL,
  inserted_at timestamp(0) NOT NULL DEFAULT timezone('UTC', now()),
  UNIQUE (tenant_id, key),
  CHECK (key ~ '^saved:[a-z0-9]+(-[a-z0-9]+)*$'),
  CHECK (jsonb_typeof(template) = 'object')
);
CREATE FUNCTION finance_ledger_chart_templates_immutable() RETURNS trigger
LANGUAGE plpgsql AS $$ BEGIN
  RAISE EXCEPTION 'A saved chart template is immutable; save a new template instead';
END $$;
CREATE TRIGGER finance_ledger_chart_templates_immutable
BEFORE UPDATE OR DELETE OR TRUNCATE ON finance_ledger_chart_templates
FOR EACH STATEMENT EXECUTE FUNCTION finance_ledger_chart_templates_immutable();
