-- Zero life is meaningful only for an opening asset already at its residual value.
ALTER TABLE finance_fixed_assets_assets
    DROP CONSTRAINT finance_fixed_assets_assets_life_months_check,
    ADD CONSTRAINT finance_fixed_assets_assets_life_months_check CHECK (
        life_months BETWEEN 1 AND 1200 OR
        (life_months = 0 AND opening_date IS NOT NULL
         AND opening_accumulated + residual_value = cost)
    );
