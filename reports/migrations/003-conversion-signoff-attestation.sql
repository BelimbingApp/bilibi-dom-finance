DROP TABLE finance_reports_conversion_signoffs;

CREATE TABLE finance_reports_conversion_signoffs (
 id bigserial PRIMARY KEY,
 tenant_id bigint NOT NULL,
 company_id bigint NOT NULL,
 report_id bigint NOT NULL REFERENCES finance_reports_issued(id),
 reviewer_reference text NOT NULL CHECK (length(btrim(reviewer_reference)) BETWEEN 1 AND 2000),
 note text NOT NULL CHECK (length(btrim(note)) BETWEEN 1 AND 8000),
 attestation text NOT NULL CHECK (length(btrim(attestation)) BETWEEN 1 AND 8000),
 signed_by_user_id bigint NOT NULL,
 signed_by_impersonator_id bigint,
 signed_at timestamp(6) NOT NULL DEFAULT (clock_timestamp() AT TIME ZONE 'UTC'),
 UNIQUE (tenant_id,company_id,report_id)
);

CREATE TRIGGER finance_reports_conversion_signoff_check BEFORE INSERT ON finance_reports_conversion_signoffs FOR EACH ROW EXECUTE FUNCTION finance_reports_check_conversion_signoff();
CREATE TRIGGER finance_reports_conversion_signoff_immutable BEFORE UPDATE OR DELETE ON finance_reports_conversion_signoffs FOR EACH ROW EXECUTE FUNCTION finance_reports_keep_issued();
CREATE TRIGGER finance_reports_conversion_signoff_no_truncate BEFORE TRUNCATE ON finance_reports_conversion_signoffs FOR EACH STATEMENT EXECUTE FUNCTION finance_reports_keep_issued();
