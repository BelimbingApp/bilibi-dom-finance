//! Core company facts shared by Finance customers and vendors. Core owns
//! authorization and masking; the caller's connection is always retained.
use crate::{FieldError, LedgerError, Result};
use bilibi_agent_api::OperationClient;
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{PartyProofs, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyFacts {
    pub id: i64,
    pub name: String,
    /// `active`, `pending`, `suspended` or `archived`; `unavailable` when
    /// Core cannot name the company for this reader (removed, or another
    /// tenant's).
    pub status: String,
    pub path: String,
    pub readable: bool,
    pub tax_id: Option<String>,
    pub registration_number: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    #[serde(default)]
    pub restricted: Vec<String>,
}
/// Reads linked company facts for Finance without taking another connection.
pub struct CompanyLinks<'a> {
    parties: &'a PartyProofs,
}
fn invalid(field: &'static str, message: impl Into<String>) -> LedgerError {
    LedgerError::Invalid(vec![FieldError::new(field, message)])
}
fn blank_to_none(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
const FACTS: [(&str, &str); 4] = [
    ("tax_id", "tax ID"),
    ("registration_number", "registration number"),
    ("email", "email"),
    ("phone", "phone"),
];
impl<'a> CompanyLinks<'a> {
    pub fn new(parties: &'a PartyProofs) -> Self {
        Self { parties }
    }
    pub async fn company_summary(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        id: i64,
    ) -> Result<CompanyFacts> {
        let named = match OrganisationId::new(id) {
            Ok(organisation) => {
                self.parties
                    .organisation_name(client, scope, organisation)
                    .await?
            }
            Err(_) => None,
        };
        let (name, status) = match named {
            Some(named) => (named.name, named.status),
            None => ("Company unavailable".into(), "unavailable".into()),
        };
        Ok(CompanyFacts {
            id,
            name,
            status,
            path: format!("/companies/{id}"),
            readable: false,
            tax_id: None,
            registration_number: None,
            email: None,
            phone: None,
            restricted: Vec::new(),
        })
    }

    /// The facts Companies view lets the reader see; without it the summary
    /// stays as it is. A restricted fact stays `None` and is named.
    pub async fn company_facts(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &mut CompanyFacts,
    ) -> Result<()> {
        let (detail, masked) = match bilibi_company::detail(client, scope, company.id).await {
            Ok(read) => read,
            Err(Error::Forbidden | Error::NotFound) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        company.readable = true;
        company.restricted = FACTS
            .iter()
            .filter(|(field, _)| masked.contains(*field))
            .map(|(field, _)| (*field).to_owned())
            .collect();
        let visible = |field: &str, value: Option<String>| {
            if masked.contains(field) {
                None
            } else {
                value.and_then(blank_to_none)
            }
        };
        company.tax_id = visible("tax_id", Some(detail.tax_id));
        company.registration_number = visible("registration_number", detail.registration_number);
        company.email = visible("email", Some(detail.email));
        company.phone = visible("phone", Some(detail.phone));
        Ok(())
    }

    /// The party an issued document freezes for a linked customer: the
    /// company's name and the facts the issuer may read, none of them masked
    /// (a mask is never frozen as a fact). The billing address is not frozen
    /// yet: Core Address offers no read on the issuing connection
    /// (TODO(platform)).
    pub async fn frozen_party(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        id: i64,
        field: &'static str,
    ) -> Result<Value> {
        let (company, masked) =
            bilibi_company::detail(client, scope, id)
                .await
                .map_err(|error| match error {
                    Error::Forbidden => invalid(
                        field,
                        "needs Companies view permission to freeze the company's details",
                    ),
                    Error::NotFound => {
                        invalid(field, "is linked to a company that no longer exists")
                    }
                    other => other.into(),
                })?;
        if let Some((_, label)) = FACTS.iter().find(|(field, _)| masked.contains(*field)) {
            return Err(invalid(
                field,
                format!("the company's {label} is restricted for you, so it cannot be frozen on the document"),
            ));
        }
        Ok(json!({
            "party_name": company.display_name(),
            "party_tax_identifier": blank_to_none(company.tax_id),
            "party_registration_identifier": company.registration_number.and_then(blank_to_none),
            "party_email": blank_to_none(company.email),
            "party_phone": blank_to_none(company.phone),
        }))
    }
}
