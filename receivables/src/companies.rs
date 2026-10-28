//! Core company identity, relationships and issue-time party facts for
//! Receivables (Bilimbi Finance Receivables), always on the caller's connection.
use crate::{invalid, store, CompanyFacts, Customer, Receivables, Result};
use bilibi_agent_api::OperationClient;
use bilibi_company::{NewCompany, RelationshipRole};
use bilibi_database::{Error, MutationTransaction};
use bilibi_party::{CalendarDate, EffectiveDates, OrganisationId, PartyId, RelationshipInput};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};

/// The Base Records kind Core Company registers.
pub(crate) const KIND: &str = "company";
/// The party columns a customer recorded before linking carries itself.
pub(crate) const PARTY: [&str; 12] = [
    "name",
    "tax_identifier",
    "registration_identifier",
    "email",
    "phone",
    "line1",
    "line2",
    "line3",
    "locality",
    "postcode",
    "admin1_code",
    "country_iso",
];
/// The linked company's id on a customer row, `None` while unlinked.
pub(crate) fn linked(row: &Value) -> Option<i64> {
    if row["record_kind"].as_str() != Some(KIND) {
        return None;
    }
    row["record_id"].as_str()?.parse().ok().filter(|id| *id > 0)
}

impl Receivables {
    /// A customer row with the company facts this reader may see.
    pub(crate) async fn customer(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        mut row: Value,
    ) -> Result<Customer> {
        let mut company = match linked(&row) {
            Some(id) => Some(self.company_summary(client, scope, id).await?),
            None => None,
        };
        if let Some(company) = &mut company {
            self.company_facts(client, scope, company).await?;
            for field in PARTY {
                row[field] = Value::Null;
            }
            row["name"] = json!(company.name);
        }
        let mut customer: Customer = store::decode(row)?;
        customer.company = company;
        Ok(customer)
    }

    /// The company as Core names it for this reader: display name and status,
    /// or unavailable when Core does not name it (removed, another tenant's).
    pub(crate) async fn company_summary(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        id: i64,
    ) -> Result<CompanyFacts> {
        Ok(bilibi_finance_ledger::CompanyLinks::new(&self.parties)
            .company_summary(client, scope, id)
            .await?)
    }
    pub(crate) async fn company_facts(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        company: &mut CompanyFacts,
    ) -> Result<()> {
        Ok(bilibi_finance_ledger::CompanyLinks::new(&self.parties)
            .company_facts(client, scope, company)
            .await?)
    }
    pub(crate) async fn frozen_party(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        id: i64,
        date: &CalendarDate,
    ) -> Result<Value> {
        let mut party = bilibi_finance_ledger::CompanyLinks::new(&self.parties)
            .frozen_party(client, scope, id, "customer_id", true)
            .await
            .map_err(|error| match error {
                bilibi_finance_ledger::LedgerError::Invalid(fields) => {
                    crate::ReceivableError::Invalid(
                        fields
                            .into_iter()
                            .map(|f| crate::FieldError {
                                field: f.field.into(),
                                line: f.line,
                                message: f.message,
                            })
                            .collect(),
                    )
                }
                other => other.into(),
            })?;
        let address = bilibi_address::billing_address(
            client,
            scope,
            PartyId::Organisation(organisation(id)?),
            date,
        )
        .await
        .map_err(|e| match e {
            Error::Forbidden => invalid(
                "customer_id",
                "needs Addresses view permission to freeze the company's billing address",
            ),
            other => other.into(),
        })?;
        let address = address.map(|a| a.address.address);
        for (field, value) in [
            ("line1", address.as_ref().and_then(|a| a.line1.clone())),
            ("line2", address.as_ref().and_then(|a| a.line2.clone())),
            ("line3", address.as_ref().and_then(|a| a.line3.clone())),
            (
                "locality",
                address.as_ref().and_then(|a| a.locality.clone()),
            ),
            (
                "postcode",
                address.as_ref().and_then(|a| a.postcode.clone()),
            ),
            (
                "admin1_code",
                address.as_ref().and_then(|a| a.admin1_code.clone()),
            ),
            (
                "country_iso",
                address.as_ref().and_then(|a| a.country_iso.clone()),
            ),
        ] {
            party[format!("party_{field}")] = json!(value);
        }
        Ok(party)
    }

    /// Whether a new invoice may name this customer: linked, available for
    /// Finance, and its company active in Core. A pending company has not
    /// been activated yet, a suspended or archived one is not open for new
    /// business; receipts and credit notes stay possible.
    pub(crate) async fn invoiceable(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        source: i64,
        customer: &Customer,
    ) -> Result<()> {
        let Some(company) = &customer.company else {
            return Err(invalid(
                "customer_id",
                "is not linked to a company; link it before invoicing",
            ));
        };
        if !customer.active {
            return Err(invalid("customer_id", "is an inactive customer"));
        }
        match company.status.as_str() {
            "active" => {
                let role = self
                    .customer_role(client, scope)
                    .await
                    .map_err(|e| match e {
                        crate::ReceivableError::Invalid(mut fields) => {
                            for field in &mut fields {
                                field.field = "customer_id".into();
                            }
                            crate::ReceivableError::Invalid(fields)
                        }
                        other => other,
                    })?;
                if self
                    .current_customer_relationship(client, scope, source, company.id, &role)
                    .await?
                    .is_none()
                {
                    return Err(invalid(
                        "customer_id",
                        "has no current customer relationship from the current company; record it in Companies before invoicing",
                    ));
                }
                Ok(())
            }
            "pending" => Err(invalid(
                "customer_id",
                "is a pending company; activate it in Companies before invoicing",
            )),
            "suspended" => Err(invalid(
                "customer_id",
                "is a suspended company; reactivate it in Companies before invoicing",
            )),
            "archived" => Err(invalid("customer_id", "is an archived company")),
            _ => Err(invalid(
                "customer_id",
                "is linked to a company that is not available",
            )),
        }
    }

    /// Proves a company may be linked as a customer of `company`: the record
    /// exists and the actor may see it (Base Records asks Core), it is not
    /// archived, it is not the company itself, and no other customer of this
    /// company links to it. Answers the stored reference's id.
    pub(crate) async fn linkable(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        company: i64,
        attrs: &Value,
        field: &str,
    ) -> Result<(i64, CompanyFacts)> {
        let id = match &attrs[field] {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse().ok(),
            Value::Null => return Err(invalid(field, "can't be blank")),
            _ => None,
        }
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid(field, "is invalid"))?;
        if id == company {
            return Err(invalid(field, "cannot be the company itself"));
        }
        let reference = self
            .records
            .check(client, scope, KIND, &id.to_string())
            .await?
            .map_err(|refusal| invalid(field, refusal.message()))?;
        let id: i64 = reference
            .id
            .parse()
            .map_err(|_| invalid(field, "is invalid"))?;
        let summary = self.company_summary(client, scope, id).await?;
        if summary.status == "archived" {
            return Err(invalid(field, "is an archived company"));
        }
        if summary.status == "unavailable" {
            return Err(invalid(field, "does not exist"));
        }
        if store::taken(
            client,
            store::CUSTOMERS,
            scope.tenant(),
            company,
            "record_id",
            &json!(id.to_string()),
        )
        .await?
        {
            return Err(invalid(field, "is already a customer of this company"));
        }
        Ok((id, summary))
    }
}

/// What the customer operation will compose, without reserving a company id.
#[derive(serde::Serialize)]
pub(super) struct Target {
    pub id: Option<i64>,
    pub company: Option<CompanyFacts>,
    pub creation: Option<Value>,
    pub relationship: Value,
}
fn organisation(id: i64) -> Result<OrganisationId> {
    OrganisationId::new(id).map_err(|_| invalid("core_company_id", "is invalid"))
}
fn company_fields(fields: Vec<bilibi_company::FieldError>) -> crate::ReceivableError {
    crate::ReceivableError::Invalid(
        fields
            .into_iter()
            .map(|e| crate::FieldError {
                field: format!("new_company.{}", e.field),
                line: None,
                message: e.message,
            })
            .collect(),
    )
}
fn new_company(attrs: &Value) -> Result<NewCompany> {
    serde_json::from_value(attrs["new_company"].clone())
        .map_err(|_| invalid("new_company", "is invalid"))
}
impl Receivables {
    async fn customer_role(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
    ) -> Result<RelationshipRole> {
        bilibi_company::relationship_role_by_code(client, scope, "customer").await
            .map_err(|e| match e {
                Error::Forbidden => invalid("base", "needs Companies view permission to check the customer relationship"),
                other => other.into(),
            })?
            .ok_or_else(|| invalid("base", "add the customer relationship role in Companies before adding or invoicing a customer"))
    }
    async fn current_customer_relationship(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        source: i64,
        target: i64,
        role: &RelationshipRole,
    ) -> Result<Option<bilibi_company::Relationship>> {
        Ok(bilibi_company::current_relationship(
            client,
            scope,
            organisation(source)?,
            PartyId::Organisation(organisation(target)?),
            role.id,
        )
        .await?)
    }
    pub(super) async fn prepare_target(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        source: i64,
        attrs: &Value,
    ) -> Result<Target> {
        let (id, company, creation) = if !attrs["new_company"].is_null() {
            if !attrs["core_company_id"].is_null() {
                return Err(invalid(
                    "core_company_id",
                    "choose an existing company or create a new one, not both",
                ));
            }
            let new = new_company(attrs)?;
            let preview = bilibi_company::preview_create(client, scope, &new)
                .await
                .map_err(|e| match e {
                    Error::Forbidden => invalid(
                        "base",
                        "needs Companies create permission to add a new company",
                    ),
                    other => other.into(),
                })?
                .map_err(company_fields)?;
            (
                None,
                None,
                Some(
                    json!({"company":new,"name":preview.company.name,"code":preview.company.code,"status":preview.company.status,"duplicate_candidates":preview.matches.candidates,"duplicates_likely":preview.matches.likely}),
                ),
            )
        } else {
            let (id, mut company) = self
                .linkable(client, scope, source, attrs, "core_company_id")
                .await?;
            self.company_facts(client, scope, &mut company).await?;
            (Some(id), Some(company), None)
        };
        let role = self.customer_role(client, scope).await?;
        let current = match id {
            Some(id) => {
                self.current_customer_relationship(client, scope, source, id, &role)
                    .await?
            }
            None => None,
        };
        if current.is_none() {
            if role.retired() {
                return Err(invalid(
                    "base",
                    "the customer relationship role is retired; restore it in Companies before adding a customer",
                ));
            }
            self.authorize_on(client, scope, source, bilibi_company::UPDATE)
                .await
                .map_err(|e| match e {
                    crate::ReceivableError::Unauthorized => invalid(
                        "base",
                        "needs Companies update permission to record the customer relationship",
                    ),
                    other => other,
                })?;
        }
        Ok(Target {
            id,
            company,
            creation,
            relationship: json!({"source_company_id":source,"role":role,"current":current,"will_assign":current.is_none()}),
        })
    }

    /// Core and Finance mutate the same audited transaction. Any subsequent
    /// refusal discards the company, relationship, customer and their audit rows.
    pub(super) async fn compose_target(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        source: i64,
        attrs: &Value,
        data: &mut Value,
    ) -> Result<()> {
        let target = if attrs["new_company"].is_null() {
            linked(data).ok_or_else(|| invalid("core_company_id", "can't be blank"))?
        } else {
            match bilibi_company::create(tx, scope, &new_company(attrs)?).await? {
                bilibi_company::CreateOutcome::Created(company) => company.id,
                bilibi_company::CreateOutcome::Refused(fields) => {
                    return Err(company_fields(fields));
                }
            }
        };
        // The newly created company must pass the same Base Records proof
        // as a selected company, on this transaction before storing its link.
        self.records
            .check(tx.sql(), scope, KIND, &target.to_string())
            .await?
            .map_err(|refusal| invalid("core_company_id", refusal.message()))?;
        let role = self.customer_role(tx.sql(), scope).await?;
        if self
            .current_customer_relationship(tx.sql(), scope, source, target, &role)
            .await?
            .is_none()
        {
            let input = RelationshipInput {
                source_organisation_id: organisation(source)?,
                target: PartyId::Organisation(organisation(target)?),
                role_id: role.id,
                dates: EffectiveDates::new(None, None).map_err(|_| Error::Internal)?,
            };
            match bilibi_company::assign_relationship(tx, scope, &self.parties, &input)
                .await
                .map_err(|e| match e {
                    bilibi_company::WriteError::Access(Error::Forbidden) => invalid(
                        "base",
                        "needs Companies update permission to record the customer relationship",
                    ),
                    bilibi_company::WriteError::Access(error) => error.into(),
                    other => invalid("base", other.to_string()),
                })? {
                bilibi_company::TypeOutcome::Saved(_) => {}
                bilibi_company::TypeOutcome::Refused(fields) => {
                    return Err(crate::ReceivableError::Invalid(
                        fields
                            .into_iter()
                            .map(|e| crate::FieldError {
                                field: "base".into(),
                                line: None,
                                message: e.message,
                            })
                            .collect(),
                    ));
                }
            }
        }
        data["record_kind"] = json!(KIND);
        data["record_id"] = json!(target.to_string());
        Ok(())
    }
}
