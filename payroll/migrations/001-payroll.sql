-- Mirrors Bilimbi finance/payroll, b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9,
-- 20261015120000_create_finance_payroll.exs (fresh schema, no legacy adoption).
-- Account and source identifiers belong to their owners, not foreign keys.
-- As in local retained-record migrations, TRUNCATE is refused as well.

CREATE TABLE finance_payroll_settings (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  net_pay_account_id bigint NOT NULL,
  claims_payable_account_id bigint NOT NULL,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL
);
CREATE UNIQUE INDEX finance_payroll_settings_company_unique ON finance_payroll_settings (company_id);
CREATE INDEX finance_payroll_settings_tenant_id_company_id_index ON finance_payroll_settings (tenant_id, company_id);

CREATE TABLE finance_payroll_settings_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  old_net_pay_account_id bigint,
  new_net_pay_account_id bigint NOT NULL,
  old_claims_payable_account_id bigint,
  new_claims_payable_account_id bigint NOT NULL,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payroll_settings_changes_before
    CHECK ((old_net_pay_account_id IS NULL) = (old_claims_payable_account_id IS NULL))
);
CREATE INDEX finance_payroll_settings_changes_tenant_id_company_id_index ON finance_payroll_settings_changes (tenant_id, company_id);

CREATE TABLE finance_payroll_mappings (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  kind varchar(20) NOT NULL,
  code varchar(60) NOT NULL,
  direction varchar(20) NOT NULL,
  expense_account_id bigint,
  liability_account_id bigint,
  inserted_at timestamp(0) NOT NULL,
  updated_at timestamp(0) NOT NULL,
  CONSTRAINT finance_payroll_mappings_kind CHECK (kind IN ('component', 'claim_category')),
  CONSTRAINT finance_payroll_mappings_direction CHECK (direction IN ('earning', 'deduction', 'employer', 'claim')),
  CONSTRAINT finance_payroll_mappings_code CHECK (code <> ''),
  CONSTRAINT finance_payroll_mappings_claim_direction CHECK ((kind = 'claim_category') = (direction = 'claim')),
  CONSTRAINT finance_payroll_mappings_accounts
    CHECK ((direction IN ('earning', 'claim')
            AND expense_account_id IS NOT NULL AND liability_account_id IS NULL)
      OR (direction = 'deduction'
            AND expense_account_id IS NULL AND liability_account_id IS NOT NULL)
      OR (direction = 'employer'
            AND expense_account_id IS NOT NULL AND liability_account_id IS NOT NULL))
);
CREATE UNIQUE INDEX finance_payroll_mappings_id_company_unique ON finance_payroll_mappings (id, company_id);
CREATE UNIQUE INDEX finance_payroll_mappings_company_key_unique ON finance_payroll_mappings (company_id, kind, code, direction);
CREATE INDEX finance_payroll_mappings_tenant_id_company_id_index ON finance_payroll_mappings (tenant_id, company_id);

CREATE TABLE finance_payroll_mapping_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  mapping_id bigint NOT NULL,
  old_expense_account_id bigint,
  new_expense_account_id bigint,
  old_liability_account_id bigint,
  new_liability_account_id bigint,
  reason varchar(500),
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payroll_mapping_changes_mapping_fkey
    FOREIGN KEY (mapping_id, company_id)
    REFERENCES finance_payroll_mappings (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_payroll_mapping_changes_after
    CHECK (new_expense_account_id IS NOT NULL OR new_liability_account_id IS NOT NULL)
);
CREATE INDEX finance_payroll_mapping_changes_tenant_id_company_id_index ON finance_payroll_mapping_changes (tenant_id, company_id);
CREATE INDEX finance_payroll_mapping_changes_mapping_id_index ON finance_payroll_mapping_changes (mapping_id);

CREATE TABLE finance_payroll_posting_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  source_kind varchar(20) NOT NULL,
  source_id bigint NOT NULL,
  source_reference varchar(100) NOT NULL,
  event varchar(20) NOT NULL,
  entry_id bigint,
  entry_number bigint,
  entry_date date,
  total numeric(20, 4),
  currency_code varchar(3),
  reason varchar(500),
  actor_user_id bigint NOT NULL,
  actor_impersonator_id bigint,
  occurred_at timestamp(6) NOT NULL,
  CONSTRAINT finance_payroll_posting_events_source_kind CHECK (source_kind IN ('payroll_run', 'claim')),
  CONSTRAINT finance_payroll_posting_events_event CHECK (event IN ('posted', 'posting_failed', 'reversed', 'reversal_failed')),
  CONSTRAINT finance_payroll_posting_events_entry
    CHECK ((event IN ('posted', 'reversed')) = (entry_id IS NOT NULL)
      AND (event IN ('posted', 'reversed')) = (entry_number IS NOT NULL)
      AND (event IN ('posted', 'reversed')) = (entry_date IS NOT NULL)
      AND (event IN ('posted', 'reversed')) = (total IS NOT NULL)
      AND (event IN ('posted', 'reversed')) = (currency_code IS NOT NULL)),
  CONSTRAINT finance_payroll_posting_events_reason CHECK (event = 'posted' OR reason IS NOT NULL),
  CONSTRAINT finance_payroll_posting_events_total CHECK (total IS NULL OR total > 0),
  CONSTRAINT finance_payroll_posting_events_currency_code CHECK (currency_code IS NULL OR currency_code ~ '^[A-Z]{3}$')
);
CREATE UNIQUE INDEX finance_payroll_posting_events_posted_unique
  ON finance_payroll_posting_events (company_id, source_kind, source_id) WHERE event = 'posted';
CREATE UNIQUE INDEX finance_payroll_posting_events_reversed_unique
  ON finance_payroll_posting_events (company_id, source_kind, source_id) WHERE event = 'reversed';
CREATE UNIQUE INDEX finance_payroll_posting_events_entry_unique
  ON finance_payroll_posting_events (company_id, entry_id) WHERE entry_id IS NOT NULL;
CREATE INDEX finance_payroll_posting_events_source_index ON finance_payroll_posting_events (company_id, source_kind, source_id);
CREATE INDEX finance_payroll_posting_events_tenant_id_company_id_index ON finance_payroll_posting_events (tenant_id, company_id);

CREATE FUNCTION finance_payroll_refuse_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance payroll keeps % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

CREATE FUNCTION finance_payroll_keep_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.tenant_id <> OLD.tenant_id OR NEW.company_id <> OLD.company_id
    OR (TG_TABLE_NAME = 'finance_payroll_mappings'
        AND (to_jsonb(NEW) ->> 'kind' <> to_jsonb(OLD) ->> 'kind'
          OR to_jsonb(NEW) ->> 'code' <> to_jsonb(OLD) ->> 'code'
          OR to_jsonb(NEW) ->> 'direction' <> to_jsonb(OLD) ->> 'direction')) THEN
    RAISE EXCEPTION 'finance payroll keeps % rows in their company and key', TG_TABLE_NAME
      USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payroll_stamp_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_payroll_record_event() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.event IN ('reversed', 'reversal_failed') AND NOT EXISTS (
    SELECT 1 FROM finance_payroll_posting_events
    WHERE tenant_id = NEW.tenant_id AND company_id = NEW.company_id
      AND source_kind = NEW.source_kind AND source_id = NEW.source_id AND event = 'posted') THEN
    RAISE EXCEPTION 'finance payroll reverses only a posted source'
      USING ERRCODE = 'restrict_violation';
  END IF;
  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE TRIGGER finance_payroll_settings_changes_append_only
  BEFORE UPDATE OR DELETE ON finance_payroll_settings_changes
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_mapping_changes_append_only
  BEFORE UPDATE OR DELETE ON finance_payroll_mapping_changes
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_posting_events_append_only
  BEFORE UPDATE OR DELETE ON finance_payroll_posting_events
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_settings_kept BEFORE DELETE ON finance_payroll_settings
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_mappings_kept BEFORE DELETE ON finance_payroll_mappings
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_settings_identity BEFORE UPDATE ON finance_payroll_settings
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_keep_identity();
CREATE TRIGGER finance_payroll_mappings_identity BEFORE UPDATE ON finance_payroll_mappings
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_keep_identity();
CREATE TRIGGER finance_payroll_settings_changes_stamp BEFORE INSERT ON finance_payroll_settings_changes
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_stamp_change();
CREATE TRIGGER finance_payroll_mapping_changes_stamp BEFORE INSERT ON finance_payroll_mapping_changes
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_stamp_change();
CREATE TRIGGER finance_payroll_posting_events_record BEFORE INSERT ON finance_payroll_posting_events
  FOR EACH ROW EXECUTE FUNCTION finance_payroll_record_event();

CREATE TRIGGER finance_payroll_settings_no_truncate BEFORE TRUNCATE ON finance_payroll_settings
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_settings_changes_no_truncate BEFORE TRUNCATE ON finance_payroll_settings_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_mappings_no_truncate BEFORE TRUNCATE ON finance_payroll_mappings
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_mapping_changes_no_truncate BEFORE TRUNCATE ON finance_payroll_mapping_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payroll_refuse_change();
CREATE TRIGGER finance_payroll_posting_events_no_truncate BEFORE TRUNCATE ON finance_payroll_posting_events
  FOR EACH STATEMENT EXECUTE FUNCTION finance_payroll_refuse_change();
