-- A customer is a Finance record linked to a Core company
-- (BelimbingApp/bilibi-dom-finance#75). The link is a Base Records reference,
-- `(record_kind, record_id)` as text, never a foreign key across the module
-- boundary. Identity, tax and registration numbers, contact details and the
-- company's status are read from Core; this table keeps only the Finance
-- code, payment terms and the Finance availability flag.
--
-- Customers recorded before this migration keep the party columns they were
-- captured with and read as "not linked" until a person links them; a new
-- customer is always linked. Once set, a link never changes: the issued
-- documents, receipts and reports behind the customer belong to that business.
ALTER TABLE finance_receivables_customers
  ADD COLUMN record_kind text,
  ADD COLUMN record_id text,
  ALTER COLUMN name DROP NOT NULL,
  ADD CONSTRAINT finance_receivables_customers_record
    CHECK ((record_kind IS NULL) = (record_id IS NULL)),
  ADD CONSTRAINT finance_receivables_customers_record_kind
    CHECK (record_kind IS NULL OR record_kind = 'company'),
  ADD CONSTRAINT finance_receivables_customers_identity
    CHECK (record_id IS NOT NULL OR name IS NOT NULL);
CREATE INDEX finance_receivables_customers_record_index
  ON finance_receivables_customers (tenant_id, record_kind, record_id);
CREATE UNIQUE INDEX finance_receivables_customers_company_record_unique
  ON finance_receivables_customers (company_id, record_kind, record_id)
  WHERE record_id IS NOT NULL;

CREATE FUNCTION finance_receivables_keep_link() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.record_id IS NOT NULL
     AND (NEW.record_id IS DISTINCT FROM OLD.record_id
          OR NEW.record_kind IS DISTINCT FROM OLD.record_kind) THEN
    RAISE EXCEPTION 'finance receivables keeps the company link of % rows', TG_TABLE_NAME
      USING ERRCODE = 'restrict_violation';
  END IF;
  RETURN NEW;
END
$$;
CREATE TRIGGER finance_receivables_customers_link BEFORE UPDATE ON finance_receivables_customers
  FOR EACH ROW EXECUTE FUNCTION finance_receivables_keep_link();
