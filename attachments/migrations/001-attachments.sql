-- finance/attachments 001: source documents kept with the records they
-- support, the end state of the Bilimbi attachments schema (b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9, migration
-- 20261017090000_create_finance_attachments).
--
-- The file's bytes live in Base Artifacts as a records-grade document; an
-- attachment row is the link from a Finance record (a bill, a payment, an
-- invoice, a manual journal, a bank statement) to that artifact, with the
-- file's name, type, size and SHA-256, the retain-until the company's setting
-- gave it, and who captured it when. The database numbers a company's
-- attachments without gaps and stamps the capture time.
--
-- The retention setting is never a column: it is the latest of the company's
-- append-only retention changes (years kept after the end of a document's
-- financial year), each with the actor, any impersonator, a reason and the
-- database's time; a company with none keeps documents for seven years. A
-- change only ever lengthens it.
--
-- An attachment's standing is its latest event: none while it stands, one
-- `replaced` naming the attachment that took its place, or one `removed`; the
-- database keeps at most one of those two per attachment. A retention
-- extension, a legal hold placed and a hold released are events too, each with
-- its reason. Every table here is append-only.

CREATE TABLE finance_attachments_retention_changes (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  change_number bigint NOT NULL,
  years integer NOT NULL,
  reason varchar(500) NOT NULL,
  changed_by_user_id bigint NOT NULL,
  changed_by_impersonator_id bigint,
  changed_at timestamp(6) NOT NULL,
  CONSTRAINT finance_attachments_retention_changes_change_number CHECK (change_number > 0),
  CONSTRAINT finance_attachments_retention_changes_years CHECK (years > 0),
  CONSTRAINT finance_attachments_retention_changes_reason CHECK (btrim(reason) <> '')
);
CREATE UNIQUE INDEX finance_attachments_retention_changes_id_company_unique
  ON finance_attachments_retention_changes (id, company_id);
CREATE UNIQUE INDEX finance_attachments_retention_changes_company_number_unique
  ON finance_attachments_retention_changes (company_id, change_number);
CREATE INDEX finance_attachments_retention_changes_tenant_company_index
  ON finance_attachments_retention_changes (tenant_id, company_id);

-- The record an attachment supports belongs to the module that names it
-- (`source`, the Source contribution's id), so the link is a module-owned
-- identifier rather than a foreign key; `source_module` is the descriptor id
-- of the module that contributed that source, which reads the record back and
-- decides whether it is final.
CREATE TABLE finance_attachments (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  attachment_number bigint NOT NULL,
  source varchar(100) NOT NULL,
  source_module varchar(255) NOT NULL,
  document_id bigint NOT NULL,
  document_label varchar(100) NOT NULL,
  document_date date NOT NULL,
  artifact_id uuid NOT NULL,
  file_name varchar(255) NOT NULL,
  content_type varchar(255) NOT NULL,
  byte_size bigint NOT NULL,
  sha256 varchar(64) NOT NULL,
  retention_years integer NOT NULL,
  retain_until timestamp(6) NOT NULL,
  captured_by_user_id bigint NOT NULL,
  captured_by_impersonator_id bigint,
  captured_at timestamp(6) NOT NULL,
  CONSTRAINT finance_attachments_attachment_number CHECK (attachment_number > 0),
  CONSTRAINT finance_attachments_source CHECK (btrim(source) <> ''),
  CONSTRAINT finance_attachments_document_id CHECK (document_id > 0),
  CONSTRAINT finance_attachments_document_label CHECK (btrim(document_label) <> ''),
  CONSTRAINT finance_attachments_file_name CHECK (btrim(file_name) <> ''),
  CONSTRAINT finance_attachments_byte_size CHECK (byte_size > 0),
  CONSTRAINT finance_attachments_sha256 CHECK (sha256 ~ '^[0-9a-f]{64}$'),
  CONSTRAINT finance_attachments_retention_years CHECK (retention_years > 0),
  CONSTRAINT finance_attachments_retain_until CHECK (retain_until > document_date)
);
CREATE UNIQUE INDEX finance_attachments_id_company_unique ON finance_attachments (id, company_id);
CREATE UNIQUE INDEX finance_attachments_company_number_unique ON finance_attachments (company_id, attachment_number);
CREATE UNIQUE INDEX finance_attachments_artifact_unique ON finance_attachments (artifact_id);
CREATE INDEX finance_attachments_document_index ON finance_attachments (company_id, source, document_id, id);
CREATE INDEX finance_attachments_tenant_company_index ON finance_attachments (tenant_id, company_id);

CREATE TABLE finance_attachment_events (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  attachment_id bigint NOT NULL,
  event varchar(30) NOT NULL,
  replacement_id bigint,
  retain_until timestamp(6),
  reason varchar(500),
  actor_user_id bigint NOT NULL,
  actor_impersonator_id bigint,
  actor_system_principal varchar(100),
  occurred_at timestamp(6) NOT NULL,
  CONSTRAINT finance_attachment_events_attachment_fkey
    FOREIGN KEY (attachment_id, company_id)
    REFERENCES finance_attachments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_attachment_events_replacement_fkey
    FOREIGN KEY (replacement_id, company_id)
    REFERENCES finance_attachments (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_attachment_events_event
    CHECK (event IN ('replaced', 'removed', 'retention_extended', 'hold_placed', 'hold_released')),
  CONSTRAINT finance_attachment_events_replacement
    CHECK ((event = 'replaced') = (replacement_id IS NOT NULL)),
  CONSTRAINT finance_attachment_events_not_self
    CHECK (replacement_id IS NULL OR replacement_id <> attachment_id),
  CONSTRAINT finance_attachment_events_retain_until
    CHECK ((event = 'retention_extended') = (retain_until IS NOT NULL)),
  CONSTRAINT finance_attachment_events_reason
    CHECK (event IN ('replaced', 'retention_extended')
      OR (reason IS NOT NULL AND btrim(reason) <> ''))
);
CREATE UNIQUE INDEX finance_attachment_events_standing_unique
  ON finance_attachment_events (attachment_id) WHERE event IN ('replaced', 'removed');
CREATE INDEX finance_attachment_events_attachment_index
  ON finance_attachment_events (company_id, attachment_id, id);
CREATE INDEX finance_attachment_events_tenant_company_index
  ON finance_attachment_events (tenant_id, company_id);

CREATE FUNCTION finance_attachments_refuse_record_change() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'finance attachments keep % rows: % is refused', TG_TABLE_NAME, TG_OP
    USING ERRCODE = 'restrict_violation';
END
$$;

-- The latest change decides the setting; with none, the default of seven
-- years applies. A change only ever lengthens the retention.
CREATE FUNCTION finance_attachments_record_retention_change() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  current integer;
BEGIN
  SELECT years INTO current FROM finance_attachments_retention_changes
  WHERE company_id = NEW.company_id ORDER BY change_number DESC LIMIT 1;

  IF NEW.years <= COALESCE(current, 7) THEN
    RAISE EXCEPTION 'retention of company % is % years and is only ever lengthened',
      NEW.company_id, COALESCE(current, 7)
      USING ERRCODE = 'restrict_violation';
  END IF;

  NEW.change_number := COALESCE(
    (SELECT max(change_number) FROM finance_attachments_retention_changes
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.changed_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE FUNCTION finance_attachments_record_attachment() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  NEW.attachment_number := COALESCE(
    (SELECT max(attachment_number) FROM finance_attachments
     WHERE company_id = NEW.company_id), 0) + 1;
  NEW.captured_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

-- A replacement is a later attachment of the same record.
CREATE FUNCTION finance_attachments_record_event() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  attachment finance_attachments%ROWTYPE;
  replacement finance_attachments%ROWTYPE;
BEGIN
  SELECT * INTO attachment FROM finance_attachments
  WHERE id = NEW.attachment_id AND company_id = NEW.company_id;

  IF NEW.event = 'replaced' AND FOUND THEN
    SELECT * INTO replacement FROM finance_attachments
    WHERE id = NEW.replacement_id AND company_id = NEW.company_id;

    IF FOUND AND (replacement.source <> attachment.source
      OR replacement.document_id <> attachment.document_id
      OR replacement.id <= attachment.id) THEN
      RAISE EXCEPTION 'attachment % is replaced only by a later attachment of the same record',
        NEW.attachment_id
        USING ERRCODE = 'restrict_violation';
    END IF;
  END IF;

  NEW.occurred_at := clock_timestamp() AT TIME ZONE 'UTC';
  RETURN NEW;
END
$$;

CREATE TRIGGER finance_attachments_retention_changes_append_only
  BEFORE UPDATE OR DELETE ON finance_attachments_retention_changes
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_refuse_record_change();
CREATE TRIGGER finance_attachments_retention_changes_no_truncate
  BEFORE TRUNCATE ON finance_attachments_retention_changes
  FOR EACH STATEMENT EXECUTE FUNCTION finance_attachments_refuse_record_change();
CREATE TRIGGER finance_attachments_append_only
  BEFORE UPDATE OR DELETE ON finance_attachments
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_refuse_record_change();
CREATE TRIGGER finance_attachments_no_truncate
  BEFORE TRUNCATE ON finance_attachments
  FOR EACH STATEMENT EXECUTE FUNCTION finance_attachments_refuse_record_change();
CREATE TRIGGER finance_attachment_events_append_only
  BEFORE UPDATE OR DELETE ON finance_attachment_events
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_refuse_record_change();
CREATE TRIGGER finance_attachment_events_no_truncate
  BEFORE TRUNCATE ON finance_attachment_events
  FOR EACH STATEMENT EXECUTE FUNCTION finance_attachments_refuse_record_change();

CREATE TRIGGER finance_attachments_retention_changes_record
  BEFORE INSERT ON finance_attachments_retention_changes
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_record_retention_change();
CREATE TRIGGER finance_attachments_record
  BEFORE INSERT ON finance_attachments
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_record_attachment();
CREATE TRIGGER finance_attachment_events_record
  BEFORE INSERT ON finance_attachment_events
  FOR EACH ROW EXECUTE FUNCTION finance_attachments_record_event();
