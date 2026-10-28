-- Core owns the permitted lengths of party facts. Keep its complete values in
-- immutable document snapshots, including multi-line billing addresses.
ALTER TABLE finance_receivables_documents
  ALTER COLUMN party_name TYPE text,
  ALTER COLUMN party_tax_identifier TYPE text,
  ALTER COLUMN party_registration_identifier TYPE text,
  ALTER COLUMN party_email TYPE text,
  ALTER COLUMN party_phone TYPE text,
  ALTER COLUMN party_line1 TYPE text,
  ALTER COLUMN party_line2 TYPE text,
  ALTER COLUMN party_line3 TYPE text,
  ALTER COLUMN party_locality TYPE text,
  ALTER COLUMN party_postcode TYPE text,
  ALTER COLUMN party_admin1_code TYPE text;
