-- Bilimbi.Finance.FixedAssets: immutable runs and corrections, without legacy claims.
CREATE TABLE finance_fixed_assets_runs (
    id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    period date NOT NULL CHECK (extract(day FROM period) = 1),
    entry_id bigint, entry_number bigint, total numeric(20,4) NOT NULL CHECK (total >= 0),
    UNIQUE (id, tenant_id, company_id), UNIQUE (company_id, period),
    CHECK ((total = 0 AND entry_id IS NULL AND entry_number IS NULL) OR
           (total > 0 AND entry_id IS NOT NULL AND entry_number IS NOT NULL))
);
CREATE TABLE finance_fixed_assets_run_items (
    id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    run_id bigint NOT NULL, asset_id bigint NOT NULL, amount numeric(20,4) NOT NULL CHECK (amount > 0),
    UNIQUE (id, tenant_id, company_id), UNIQUE (run_id, asset_id),
    FOREIGN KEY (run_id, tenant_id, company_id) REFERENCES finance_fixed_assets_runs (id, tenant_id, company_id),
    FOREIGN KEY (asset_id, tenant_id, company_id) REFERENCES finance_fixed_assets_assets (id, tenant_id, company_id)
);
CREATE TABLE finance_fixed_assets_run_exceptions (
    id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    run_id bigint NOT NULL, asset_id bigint NOT NULL,
    UNIQUE (id, tenant_id, company_id), UNIQUE (run_id, asset_id),
    FOREIGN KEY (run_id, tenant_id, company_id) REFERENCES finance_fixed_assets_runs (id, tenant_id, company_id),
    FOREIGN KEY (asset_id, tenant_id, company_id) REFERENCES finance_fixed_assets_assets (id, tenant_id, company_id)
);
CREATE TABLE finance_fixed_assets_events (
    id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
    recorded_by_user_id bigint NOT NULL CHECK (recorded_by_user_id > 0),
    recorded_by_impersonator_id bigint, recorded_at timestamp(6) NOT NULL,
    event_number bigint NOT NULL CHECK (event_number > 0), asset_id bigint, run_id bigint,
    action varchar(20) NOT NULL CHECK (action IN ('run_reversed','disposed','disposal_reversed','acquisition_voided')),
    effective_date date NOT NULL, entry_id bigint, entry_number bigint,
    reverses_event_id bigint, proceeds numeric(20,4), accumulated numeric(20,4),
    reason varchar(500) NOT NULL CHECK (btrim(reason) <> ''),
    UNIQUE (id, tenant_id, company_id), UNIQUE (company_id, event_number),
    CHECK ((action = 'run_reversed' AND run_id IS NOT NULL AND asset_id IS NULL AND reverses_event_id IS NULL AND proceeds IS NULL AND accumulated IS NULL)
        OR (action = 'disposed' AND asset_id IS NOT NULL AND run_id IS NULL AND reverses_event_id IS NULL AND proceeds IS NOT NULL AND accumulated IS NOT NULL AND proceeds >= 0 AND accumulated >= 0)
        OR (action = 'disposal_reversed' AND asset_id IS NOT NULL AND run_id IS NULL AND reverses_event_id IS NOT NULL AND proceeds IS NULL AND accumulated IS NULL)
        OR (action = 'acquisition_voided' AND asset_id IS NOT NULL AND run_id IS NULL AND reverses_event_id IS NULL AND proceeds IS NULL AND accumulated IS NOT NULL AND accumulated >= 0)),
    CHECK ((entry_id IS NOT NULL AND entry_number IS NOT NULL) OR
        (action = 'acquisition_voided' AND accumulated = 0 AND entry_id IS NULL AND entry_number IS NULL)),
    FOREIGN KEY (run_id, tenant_id, company_id) REFERENCES finance_fixed_assets_runs (id, tenant_id, company_id),
    FOREIGN KEY (asset_id, tenant_id, company_id) REFERENCES finance_fixed_assets_assets (id, tenant_id, company_id),
    FOREIGN KEY (reverses_event_id, tenant_id, company_id) REFERENCES finance_fixed_assets_events (id, tenant_id, company_id)
);
CREATE UNIQUE INDEX finance_fixed_assets_events_run_reversal ON finance_fixed_assets_events (run_id) WHERE action = 'run_reversed';
CREATE UNIQUE INDEX finance_fixed_assets_events_void ON finance_fixed_assets_events (asset_id) WHERE action = 'acquisition_voided';
CREATE UNIQUE INDEX finance_fixed_assets_events_disposal_reversal ON finance_fixed_assets_events (reverses_event_id) WHERE reverses_event_id IS NOT NULL;

CREATE FUNCTION finance_fixed_assets_activity_stamp() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(NEW.company_id);
    NEW.recorded_at := clock_timestamp() AT TIME ZONE 'UTC';
    IF TG_TABLE_NAME = 'finance_fixed_assets_events' THEN
        SELECT coalesce(max(event_number),0)+1 INTO NEW.event_number FROM finance_fixed_assets_events WHERE company_id=NEW.company_id;
    END IF;
    RETURN NEW;
END $$;
CREATE FUNCTION finance_fixed_assets_check_run() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE checked_id bigint; expected numeric;
BEGIN
    IF TG_TABLE_NAME = 'finance_fixed_assets_runs' THEN checked_id := NEW.id;
    ELSE checked_id := NEW.run_id; END IF;
    SELECT total INTO expected FROM finance_fixed_assets_runs WHERE id=checked_id;
    IF (SELECT coalesce(sum(amount),0) FROM finance_fixed_assets_run_items WHERE run_id=checked_id) <> expected THEN
        RAISE EXCEPTION 'fixed asset run total differs from items';
    END IF;
    RETURN NULL;
END $$;
CREATE FUNCTION finance_fixed_assets_item() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM finance_fixed_assets_runs WHERE id=NEW.run_id
        AND xmin::text::bigint = (pg_current_xact_id()::text::bigint & 4294967295)) THEN
        RAISE EXCEPTION 'fixed asset run already recorded';
    END IF;
    RETURN NEW;
END $$;
CREATE FUNCTION finance_fixed_assets_event() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE previous_action text; original_date date;
BEGIN
    PERFORM pg_advisory_xact_lock(NEW.company_id);
    IF NEW.asset_id IS NOT NULL THEN
        SELECT action INTO previous_action FROM finance_fixed_assets_events
        WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND asset_id=NEW.asset_id ORDER BY event_number DESC LIMIT 1;
    END IF;
    IF NEW.action='disposed' THEN
        IF previous_action='disposed' THEN RAISE EXCEPTION 'fixed asset already disposed'; END IF;
        IF EXISTS (SELECT 1 FROM finance_fixed_assets_events WHERE asset_id=NEW.asset_id AND action='acquisition_voided') THEN
            RAISE EXCEPTION 'fixed asset acquisition voided';
        END IF;
    ELSIF NEW.action='acquisition_voided' THEN
        IF previous_action='disposed' THEN RAISE EXCEPTION 'fixed asset acquisition void is invalid'; END IF;
    ELSIF NEW.action='disposal_reversed' THEN
        IF previous_action IS DISTINCT FROM 'disposed' OR NOT EXISTS (
            SELECT 1 FROM finance_fixed_assets_events WHERE id=NEW.reverses_event_id AND asset_id=NEW.asset_id
            AND action='disposed' AND effective_date<=NEW.effective_date
            AND id=(SELECT id FROM finance_fixed_assets_events WHERE asset_id=NEW.asset_id ORDER BY event_number DESC LIMIT 1)
        ) THEN RAISE EXCEPTION 'fixed asset disposal reversal is invalid'; END IF;
    ELSE
        SELECT (period+interval '1 month'-interval '1 day')::date INTO original_date
        FROM finance_fixed_assets_runs WHERE id=NEW.run_id AND entry_id IS NOT NULL;
        IF original_date IS NULL OR NEW.effective_date<original_date THEN RAISE EXCEPTION 'fixed asset run reversal is invalid'; END IF;
    END IF;
    RETURN NEW;
END $$;
DO $$ DECLARE name text; BEGIN
    FOREACH name IN ARRAY ARRAY['runs','run_items','run_exceptions','events'] LOOP
        name := 'finance_fixed_assets_' || name;
        EXECUTE format('CREATE INDEX %I ON %I (tenant_id, company_id)',name || '_scope',name);
        EXECUTE format('CREATE TRIGGER %I BEFORE UPDATE OR DELETE ON %I FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_refuse()',name || '_immutable',name);
        EXECUTE format('CREATE TRIGGER %I BEFORE TRUNCATE ON %I FOR EACH STATEMENT EXECUTE FUNCTION finance_fixed_assets_refuse()',name || '_retain',name);
        EXECUTE format('CREATE TRIGGER %I BEFORE INSERT ON %I FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_activity_stamp()',name || '_stamp',name);
    END LOOP;
END $$;
CREATE TRIGGER finance_fixed_assets_run_items_closed BEFORE INSERT ON finance_fixed_assets_run_items FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_item();
CREATE TRIGGER finance_fixed_assets_run_exceptions_closed BEFORE INSERT ON finance_fixed_assets_run_exceptions FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_item();
CREATE CONSTRAINT TRIGGER finance_fixed_assets_runs_total AFTER INSERT ON finance_fixed_assets_runs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_check_run();
CREATE CONSTRAINT TRIGGER finance_fixed_assets_run_items_total AFTER INSERT ON finance_fixed_assets_run_items DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_check_run();
CREATE TRIGGER finance_fixed_assets_events_transition BEFORE INSERT ON finance_fixed_assets_events FOR EACH ROW EXECUTE FUNCTION finance_fixed_assets_event();
