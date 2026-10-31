-- Accountant attestation is a distinct, immutable event against an issued copy.
CREATE TABLE finance_reports_conversion_signoffs (
 id bigserial PRIMARY KEY,
 tenant_id bigint NOT NULL,
 company_id bigint NOT NULL,
 report_id bigint NOT NULL REFERENCES finance_reports_issued(id),
 reviewer_reference text NOT NULL CHECK (length(btrim(reviewer_reference)) BETWEEN 1 AND 2000),
 note text NOT NULL CHECK (length(btrim(note)) BETWEEN 1 AND 8000),
 signed_by_user_id bigint NOT NULL,
 signed_by_impersonator_id bigint,
 signed_at timestamp(6) NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE (tenant_id,company_id,report_id)
);
CREATE FUNCTION finance_reports_check_conversion_signoff() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM finance_reports_issued WHERE id=NEW.report_id AND tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND kind='conversion') THEN
  RAISE EXCEPTION 'sign-off must name an issued conversion report of the same company';
 END IF;
 NEW.signed_at := clock_timestamp() AT TIME ZONE 'UTC';
 RETURN NEW;
END;
$$;
CREATE TRIGGER finance_reports_conversion_signoff_check BEFORE INSERT ON finance_reports_conversion_signoffs FOR EACH ROW EXECUTE FUNCTION finance_reports_check_conversion_signoff();
CREATE TRIGGER finance_reports_conversion_signoff_immutable BEFORE UPDATE OR DELETE ON finance_reports_conversion_signoffs FOR EACH ROW EXECUTE FUNCTION finance_reports_keep_issued();
CREATE TRIGGER finance_reports_conversion_signoff_no_truncate BEFORE TRUNCATE ON finance_reports_conversion_signoffs FOR EACH STATEMENT EXECUTE FUNCTION finance_reports_keep_issued();
