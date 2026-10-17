-- finance/einvoice 002: the documents a consolidated document covers. The end
-- state of the Bilimbi e-invoice consolidation schema (b-dom-finance
-- 4abfaf829321f8540581d833752a086b21b45ba9).
--
-- A consolidated document reports, in one document to the authority, the
-- sales of a period that no buyer asked an individual document for. Each row
-- names one covered (source) document and a line of the consolidated document
-- that carries it (a source whose lines are taxed differently is carried by
-- one line per tax). Rows are append-only, like the documents themselves, and
-- are written only in the transaction that records the consolidated document.
--
-- PostgreSQL enforces what the facade checks:
--  - a source is covered by one consolidated document at a time: a second is
--    refused unless every earlier one was refused, found invalid or cancelled
--    at the authority, which releases its sources;
--  - a source that was ever submitted on its own is never covered, and a
--    covered source is never submitted on its own while its consolidated
--    document holds it; a consolidated document that released its sources is
--    never submitted again;
--  - a consolidated document is never itself a source, and its sources belong
--    to its company and environment;
--  - at commit, a consolidated document's totals equal its sources' totals.

CREATE TABLE finance_einvoice_consolidated_sources (
  id bigserial PRIMARY KEY,
  tenant_id bigint NOT NULL,
  company_id bigint NOT NULL,
  document_id bigint NOT NULL,
  source_document_id bigint NOT NULL,
  line_number integer NOT NULL,
  CONSTRAINT finance_einvoice_consolidated_sources_document_fkey
    FOREIGN KEY (document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_consolidated_sources_source_fkey
    FOREIGN KEY (source_document_id, company_id)
    REFERENCES finance_einvoice_documents (id, company_id) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_consolidated_sources_line_fkey
    FOREIGN KEY (document_id, line_number)
    REFERENCES finance_einvoice_document_lines (document_id, line_number) ON DELETE RESTRICT,
  CONSTRAINT finance_einvoice_consolidated_sources_not_own_source
    CHECK (source_document_id <> document_id)
);
CREATE UNIQUE INDEX finance_einvoice_consolidated_sources_unique
  ON finance_einvoice_consolidated_sources (document_id, line_number, source_document_id);
CREATE INDEX finance_einvoice_consolidated_sources_source_index
  ON finance_einvoice_consolidated_sources (company_id, source_document_id);

CREATE FUNCTION finance_einvoice_refuse_covered_source() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF (SELECT environment FROM finance_einvoice_documents WHERE id = NEW.source_document_id)
     IS DISTINCT FROM
     (SELECT environment FROM finance_einvoice_documents WHERE id = NEW.document_id) THEN
    RAISE EXCEPTION 'document % is in another environment than consolidated document %',
      NEW.source_document_id, NEW.document_id
      USING ERRCODE = 'check_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_einvoice_submissions WHERE document_id = NEW.source_document_id
  ) THEN
    RAISE EXCEPTION 'document % was submitted on its own and cannot be consolidated',
      NEW.source_document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_einvoice_consolidated_sources
    WHERE document_id = NEW.source_document_id OR source_document_id = NEW.document_id
  ) THEN
    RAISE EXCEPTION 'a consolidated document is never a source'
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_einvoice_consolidated_sources c
    WHERE c.source_document_id = NEW.source_document_id
      AND c.document_id <> NEW.document_id
      AND NOT EXISTS (
        SELECT 1 FROM finance_einvoice_status_events e
        WHERE e.document_id = c.document_id
          AND e.event IN ('refused', 'invalid', 'cancelled'))
  ) THEN
    RAISE EXCEPTION 'document % is already consolidated', NEW.source_document_id
      USING ERRCODE = 'unique_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_einvoice_refuse_consolidated_submission() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM finance_einvoice_consolidated_sources c
    WHERE c.source_document_id = NEW.document_id
      AND NOT EXISTS (
        SELECT 1 FROM finance_einvoice_status_events e
        WHERE e.document_id = c.document_id
          AND e.event IN ('refused', 'invalid', 'cancelled'))
  ) THEN
    RAISE EXCEPTION 'document % is consolidated and cannot be submitted on its own',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  IF EXISTS (
    SELECT 1 FROM finance_einvoice_consolidated_sources WHERE document_id = NEW.document_id
  ) AND EXISTS (
    SELECT 1 FROM finance_einvoice_status_events
    WHERE document_id = NEW.document_id AND event IN ('refused', 'invalid', 'cancelled')
  ) THEN
    RAISE EXCEPTION 'consolidated document % released its sources and is not submitted again',
      NEW.document_id
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN NEW;
END
$$;

CREATE FUNCTION finance_einvoice_check_consolidation() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
  net numeric;
  tax numeric;
BEGIN
  SELECT sum(s.line_extension_amount), sum(s.tax_amount) INTO net, tax
  FROM finance_einvoice_documents s
  WHERE s.id IN (SELECT source_document_id FROM finance_einvoice_consolidated_sources
                 WHERE document_id = NEW.id);

  IF net IS NOT NULL AND (net <> NEW.line_extension_amount OR tax <> NEW.tax_amount) THEN
    RAISE EXCEPTION 'consolidated document % totals differ from its sources', NEW.id
      USING ERRCODE = 'check_violation';
  END IF;

  RETURN NULL;
END
$$;

CREATE TRIGGER finance_einvoice_consolidated_sources_append_only
  BEFORE UPDATE OR DELETE ON finance_einvoice_consolidated_sources
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_consolidated_sources_no_truncate
  BEFORE TRUNCATE ON finance_einvoice_consolidated_sources
  FOR EACH STATEMENT EXECUTE FUNCTION finance_einvoice_refuse_change();
CREATE TRIGGER finance_einvoice_consolidated_sources_recorded_document
  BEFORE INSERT ON finance_einvoice_consolidated_sources
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_recorded_line();
CREATE TRIGGER finance_einvoice_consolidated_sources_covered
  BEFORE INSERT ON finance_einvoice_consolidated_sources
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_covered_source();
CREATE CONSTRAINT TRIGGER finance_einvoice_documents_match_sources
  AFTER INSERT ON finance_einvoice_documents
  DEFERRABLE INITIALLY DEFERRED
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_check_consolidation();
CREATE TRIGGER finance_einvoice_submissions_consolidated
  BEFORE INSERT ON finance_einvoice_submissions
  FOR EACH ROW EXECUTE FUNCTION finance_einvoice_refuse_consolidated_submission();
