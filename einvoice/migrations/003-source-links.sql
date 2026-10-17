-- finance/einvoice 003: the source transaction a fiscal document is the tax
-- document of. The end state of the Bilimbi e-invoice source-link schema
-- (b-dom-finance 4abfaf829321f8540581d833752a086b21b45ba9).
--
-- A document issued for a transaction another module already recognised in
-- the ledger (a receivables invoice, a payables self-billed bill) carries one
-- link per source: the source module (its descriptor id), the record in that
-- module's terms, and the ledger entry that record posted, as the ledger
-- names that entry's owner. A linked document never posts to the ledger
-- itself: its accounting lives with its source, and so does its correction. A
-- consolidated document carries its sources' links.
--
-- PostgreSQL enforces what the facade checks:
--  - a link is written only in the transaction that records its document, and
--    a document links each source once;
--  - a source has one standing fiscal document: a second link to it is
--    refused while another document that links it stands, unless the new
--    document is a consolidated document covering that one. A document stands
--    when its latest status is `submitted`, `valid`, `rejected` or
--    `consolidated`, and also while an attempt to submit it may be held by the
--    authority: a stored submission recorded after its latest status event (in
--    flight, or never answered), which an `issued` document with any
--    submission has;
--  - a document is not submitted while another standing document links one of
--    its sources, by the same rule;
--  - links are append-only.

CREATE TABLE finance_einvoice_source_links (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  source_module varchar(100) NOT NULL,
  source_record varchar(100) NOT NULL,
  ledger_entry_id bigint NOT NULL,
  CONSTRAINT finance_einvoice_source_links_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_source_links_source_module
    CHECK (source_module ~ '^[a-z][a-z0-9_]*/[a-z][a-z0-9_]*$'),
  CONSTRAINT finance_einvoice_source_links_source_record
    CHECK (source_record ~ '^[a-z][a-z0-9_]*:[1-9][0-9]*$'),
  CONSTRAINT finance_einvoice_source_links_ledger_entry_id CHECK (ledger_entry_id > 0)
);
CREATE UNIQUE INDEX finance_einvoice_source_links_unique
  ON finance_einvoice_source_links (document_id, source_module, source_record);
CREATE INDEX finance_einvoice_source_links_source_index
  ON finance_einvoice_source_links (company_id, source_module, source_record);

-- The document that stands as the fiscal document of `link`'s source, other
-- than `document`, or NULL.
CREATE FUNCTION finance_einvoice_standing_source_link(
  link finance_einvoice_source_links, document bigint
) RETURNS bigint
LANGUAGE sql STABLE AS $$
  SELECT other.document_id
  FROM finance_einvoice_source_links other
  WHERE other.company_id = link.company_id
    AND other.source_module = link.source_module
    AND other.source_record = link.source_record
    AND other.document_id <> document
    AND (
      (SELECT e.event FROM finance_einvoice_status_events e
       WHERE e.document_id = other.document_id
         AND e.event IN ('issued', 'submitted', 'refused', 'valid', 'invalid', 'cancelled',
                         'rejected', 'consolidated', 'released')
       ORDER BY e.sequence DESC LIMIT 1) IN ('submitted', 'valid', 'rejected', 'consolidated')
      OR EXISTS (
        SELECT 1 FROM finance_einvoice_submissions s
        WHERE s.document_id = other.document_id
          AND s.recorded_at > (
            SELECT e.recorded_at FROM finance_einvoice_status_events e
            WHERE e.document_id = other.document_id
              AND e.event IN ('issued', 'submitted', 'refused', 'valid', 'invalid', 'cancelled',
                              'rejected', 'consolidated', 'released')
            ORDER BY e.sequence DESC LIMIT 1)))
    AND NOT EXISTS (
      SELECT 1 FROM finance_einvoice_consolidated_sources c
      WHERE c.document_id = document AND c.source_document_id = other.document_id)
  ORDER BY other.document_id
  LIMIT 1
$$;

CREATE FUNCTION finance_einvoice_refuse_second_source_link() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  standing bigint;
BEGIN
  standing := finance_einvoice_standing_source_link(NEW, NEW.document_id);

  IF standing IS NOT NULL THEN
    RAISE EXCEPTION '% % is the source of document %, which stands',
      NEW.source_module, NEW.source_record, standing
      USING ERRCODE = 'unique_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_einvoice_refuse_linked_submission() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  link finance_einvoice_source_links%ROWTYPE;
  standing bigint;
BEGIN
  FOR link IN
    SELECT * FROM finance_einvoice_source_links WHERE document_id = NEW.document_id
  LOOP
    standing := finance_einvoice_standing_source_link(link, NEW.document_id);

    IF standing IS NOT NULL THEN
      RAISE EXCEPTION 'document % is not submitted: % % is the source of document %, which stands',
        NEW.document_id, link.source_module, link.source_record, standing
        USING ERRCODE = 'restrict_violation';
    END IF;
  END LOOP;

  RETURN NEW;
END
$$;

CREATE TRIGGER finance_einvoice_source_links_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_source_links
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_source_links_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_source_links
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_source_links_recorded_document
  BEFORE INSERT ON finance_einvoice_source_links
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_recorded_line();
CREATE TRIGGER finance_einvoice_source_links_standing
  BEFORE INSERT ON finance_einvoice_source_links
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_second_source_link();
CREATE TRIGGER finance_einvoice_submissions_linked
  BEFORE INSERT ON finance_einvoice_submissions
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_linked_submission();
