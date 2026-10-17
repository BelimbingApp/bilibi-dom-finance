-- Mirrors Bilimbi Finance.Reports.Migrations.CreateIssuedReports: issued
-- report snapshots, stamped by the database and immutable once inserted.

CREATE TABLE finance_reports_issued (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  kind varchar(40) NOT NULL,
  implementation_version varchar(40) NOT NULL,
  through_entry_number bigint NOT NULL CONSTRAINT finance_reports_issued_cutoff CHECK (through_entry_number >= 0),
  content jsonb NOT NULL,
  chart jsonb NOT NULL,
  layout jsonb NOT NULL,
  membership jsonb NOT NULL,
  issued_by_user_id bigint NOT NULL,
  issued_by_impersonator_id bigint,
  issued_at timestamp(6) NOT NULL
);
CREATE INDEX finance_reports_issued_scope ON finance_reports_issued (tenant_id, company_id, id);

CREATE FUNCTION finance_reports_keep_issued() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    NEW.issued_at := clock_timestamp() AT TIME ZONE 'UTC';
    RETURN NEW;
  END IF;
  RAISE EXCEPTION 'issued reports are immutable';
END;
$$;
CREATE TRIGGER finance_reports_stamp BEFORE INSERT ON finance_reports_issued
  FOR EACH ROW EXECUTE FUNCTION finance_reports_keep_issued();
CREATE TRIGGER finance_reports_immutable BEFORE UPDATE OR DELETE ON finance_reports_issued
  FOR EACH ROW EXECUTE FUNCTION finance_reports_keep_issued();
CREATE TRIGGER finance_reports_no_truncate BEFORE TRUNCATE ON finance_reports_issued
  FOR EACH STATEMENT EXECUTE FUNCTION finance_reports_keep_issued();
