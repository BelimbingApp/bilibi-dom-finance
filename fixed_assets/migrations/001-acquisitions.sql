-- Bilimbi.Finance.FixedAssets.Migrations.CreateFixedAssets, NX1 tables only.
CREATE TABLE finance_fixed_assets_categories (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    name varchar(200) NOT NULL CHECK (btrim(name) <> ''),
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    UNIQUE (id, tenant_id, company_id), UNIQUE (company_id, name)
);
CREATE INDEX finance_fixed_assets_categories_scope ON finance_fixed_assets_categories (tenant_id, company_id);
CREATE TABLE finance_fixed_assets_category_revisions (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    category_id bigint NOT NULL,
    cost_account_id bigint NOT NULL, accumulated_account_id bigint NOT NULL,
    expense_account_id bigint NOT NULL, gain_account_id bigint NOT NULL, loss_account_id bigint NOT NULL,
    method varchar(20) NOT NULL CHECK (method IN ('straight_line', 'reducing_balance')),
    life_months integer NOT NULL CHECK (life_months BETWEEN 1 AND 1200),
    annual_rate numeric(7,4), reason varchar(500) NOT NULL CHECK (btrim(reason) <> ''),
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    UNIQUE (id, tenant_id, company_id),
    CHECK ((method = 'straight_line' AND annual_rate IS NULL) OR
           (method = 'reducing_balance' AND annual_rate IS NOT NULL AND annual_rate > 0 AND annual_rate <= 100)),
    FOREIGN KEY (category_id, tenant_id, company_id) REFERENCES finance_fixed_assets_categories (id, tenant_id, company_id) ON DELETE RESTRICT
);
CREATE INDEX finance_fixed_assets_category_revisions_scope ON finance_fixed_assets_category_revisions (tenant_id, company_id);
CREATE TABLE finance_fixed_assets_assets (
    id bigserial PRIMARY KEY,
    tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    asset_number bigint NOT NULL CHECK (asset_number > 0), category_revision_id bigint NOT NULL,
    description varchar(500) NOT NULL CHECK (btrim(description) <> ''),
    acquisition_date date NOT NULL, in_service_date date NOT NULL CHECK (in_service_date >= acquisition_date),
    cost numeric(20,4) NOT NULL CHECK (cost > 0),
    residual_value numeric(20,4) NOT NULL CHECK (residual_value >= 0 AND residual_value <= cost),
    method varchar(20) NOT NULL CHECK (method IN ('straight_line', 'reducing_balance')),
    life_months integer NOT NULL CHECK (life_months BETWEEN 1 AND 1200), annual_rate numeric(7,4),
    entry_id bigint NOT NULL, entry_number bigint NOT NULL,
    entry_line_number integer NOT NULL CHECK (entry_line_number > 0),
    bill_id bigint, bill_line_number integer,
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    UNIQUE (id, tenant_id, company_id), UNIQUE (company_id, asset_number),
    UNIQUE (company_id, entry_id, entry_line_number),
    CHECK ((bill_id IS NULL AND bill_line_number IS NULL) OR
           (bill_id IS NOT NULL AND bill_line_number IS NOT NULL AND bill_id > 0 AND bill_line_number > 0)),
    CHECK ((method = 'straight_line' AND annual_rate IS NULL) OR
           (method = 'reducing_balance' AND annual_rate IS NOT NULL AND annual_rate > 0 AND annual_rate <= 100)),
    FOREIGN KEY (category_revision_id, tenant_id, company_id) REFERENCES finance_fixed_assets_category_revisions (id, tenant_id, company_id) ON DELETE RESTRICT
);
CREATE INDEX finance_fixed_assets_assets_scope ON finance_fixed_assets_assets (tenant_id, company_id);

CREATE FUNCTION finance_fixed_assets_refuse() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'fixed assets retain every record permanently'; END $$;
CREATE FUNCTION finance_fixed_assets_stamp() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(NEW.company_id);
    NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
    IF TG_TABLE_NAME = 'finance_fixed_assets_assets' THEN
        SELECT coalesce(max(asset_number), 0) + 1 INTO NEW.asset_number
        FROM finance_fixed_assets_assets WHERE company_id = NEW.company_id;
    END IF;
    RETURN NEW;
END $$;
DO $$ DECLARE name text; BEGIN
    FOREACH name IN ARRAY ARRAY['categories','category_revisions','assets'] LOOP
        name := 'finance_fixed_assets_' || name;
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_refuse()', name || '_immutable', name);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I FOR EACH STATEMENT EXECUTE FUNCTION finance_fixed_assets_refuse()', name || '_retain', name);
        EXECUTE format('CREATE TRIGGER %I BEFORE INSERT ON %I FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_stamp()', name || '_stamp', name);
    END LOOP;
END $$;
