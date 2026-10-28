-- A vendor can be registered before the company's book is opened, as a
-- customer can. Optional account defaults keep their account foreign keys;
-- facade authorization and the company write lock still prove the scope.
ALTER TABLE finance_payables_vendors
  DROP CONSTRAINT finance_payables_vendors_company_id_fkey;
