-- Finance-owned copies of People payroll and claims approval events v1.
-- No foreign keys or reads into People; delivery and audit commit together.
CREATE TABLE finance_payroll_source_runs (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    run_id bigint NOT NULL CHECK (run_id > 0),
    reference text NOT NULL,
    currency_code text NOT NULL CHECK (currency_code ~ '^[A-Z]{3}$'),
    pay_date date NOT NULL,
    event_key text NOT NULL,
    UNIQUE (tenant_id, company_id, run_id),
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
CREATE TABLE finance_payroll_source_components (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    run_id bigint NOT NULL,
    code text NOT NULL CHECK (code <> ''),
    direction text NOT NULL CHECK (direction IN ('earning','deduction','employer')),
    amount text NOT NULL CHECK (amount ~ '^[0-9]+(\.[0-9]{1,12})?$'),
    UNIQUE (tenant_id, company_id, run_id, code, direction),
    FOREIGN KEY (tenant_id, company_id, run_id)
        REFERENCES finance_payroll_source_runs (tenant_id, company_id, run_id)
);
CREATE TABLE finance_payroll_source_claims (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL,
    company_id bigint NOT NULL,
    claim_id bigint NOT NULL CHECK (claim_id > 0),
    reference text NOT NULL,
    currency_code text NOT NULL CHECK (currency_code ~ '^[A-Z]{3}$'),
    approved_on date NOT NULL,
    amount text NOT NULL CHECK (amount ~ '^[0-9]+(\.[0-9]{1,2})?$'),
    category_code text NOT NULL,
    reimbursed boolean NOT NULL,
    event_key text NOT NULL,
    UNIQUE (tenant_id, company_id, claim_id),
    FOREIGN KEY (tenant_id, company_id) REFERENCES companies (tenant_id, id)
);
