-- Immutable source-period evidence, not historical postings or submissions.
CREATE TABLE finance_reports_source_periods (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  kind text NOT NULL CHECK (kind IN ('comparative', 'movement', 'sst', 'payroll')),
  source_reference text NOT NULL,
  source_system text NOT NULL,
  currency_code text NOT NULL,
  accounting_timezone text NOT NULL,
  conversion_date date NOT NULL,
  opening_entry_id bigint NOT NULL,
  period_start date NOT NULL,
  period_end date NOT NULL,
  facts jsonb NOT NULL,
  recorded_by_user_id bigint NOT NULL,
  recorded_by_impersonator_id bigint,
  recorded_at timestamp(6) NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC'),
  UNIQUE (tenant_id, company_id, kind, source_reference),
  CHECK (period_start <= period_end AND period_end < conversion_date)
);
CREATE INDEX finance_reports_source_periods_scope ON finance_reports_source_periods (tenant_id, company_id, kind, id);
CREATE FUNCTION finance_reports_keep_source_period() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'source-period records are immutable';
END;
$$;
CREATE TRIGGER finance_reports_source_period_immutable BEFORE UPDATE OR DELETE ON finance_reports_source_periods
  FOR EACH ROW EXECUTE FUNCTION finance_reports_keep_source_period();
CREATE TRIGGER finance_reports_source_period_no_truncate BEFORE TRUNCATE ON finance_reports_source_periods
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reports_keep_source_period();
