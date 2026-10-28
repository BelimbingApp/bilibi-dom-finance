//! What Receivables reads from Core Company about a linked customer, on the
//! connection the caller already holds, never from Core's tables. A reviewed
//! write previews on a proof connection, which Core answers with a company's
//! name and status (`PartyProofs::organisation_name`); its tax, registration
//! and contact facts need a full connection (`bilibi_company::detail`), so
//! they are read on pooled reads and at commit, where the issue freezes them.
//! TODO(platform): company facts, a by-code relationship-role read, a
//! current-relationship read and the billing address on a caller's
//! connection (BelimbingApp/bilibi#231), so a preview shows what the commit
//! will freeze and Finance composes the link in its own transaction.
use crate::{invalid, store, Customer, CustomerCompany, Receivables, Result};
use bilibi_agent_api::OperationClient;
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

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
/// Core's facts an issued document freezes beside the name, with the words
/// a refusal uses for each.
const FACTS: [(&str, &str); 4] = [
    ("tax_id", "tax ID"),
    ("registration_number", "registration number"),
    ("email", "email"),
    ("phone", "phone"),
];

/// The linked company's id on a customer row, `None` while unlinked.
pub(crate) fn linked(row: &Value) -> Option<i64> {
    if row["record_kind"].as_str() != Some(KIND) {
        return None;
    }
    row["record_id"].as_str()?.parse().ok().filter(|id| *id > 0)
}

fn blank_to_none(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

impl Receivables {
    /// A customer row as a facade answer. The linked company carries what a
    /// proof connection gives: its name and status; `company_facts` adds the
    /// rest on a full connection.
    pub(crate) async fn customer(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        mut row: Value,
    ) -> Result<Customer> {
        let company = match linked(&row) {
            Some(id) => Some(self.company_summary(client, scope, id).await?),
            None => None,
        };
        if let Some(company) = &company {
            for field in PARTY {
                row[field] = Value::Null;
            }
            row["name"] = json!(company.name);
        }
        let mut customer: Customer = store::decode(row)?;
        customer.company = company;
        Ok(customer)
    }

    /// A customer with its company's facts: pooled reads and commits, which
    /// hold a full connection.
    pub(crate) async fn customer_with_facts(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        row: Value,
    ) -> Result<Customer> {
        let mut customer = self.customer(client, scope, row).await?;
        if let Some(company) = &mut customer.company {
            self.company_facts(client, scope, company).await?;
        }
        Ok(customer)
    }

    /// The company as Core names it for this reader: display name and status,
    /// or unavailable when Core does not name it (removed, another tenant's).
    pub(crate) async fn company_summary(
        &self,
        client: &dyn OperationClient,
        scope: &Scope,
        id: i64,
    ) -> Result<CustomerCompany> {
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
        Ok(CustomerCompany {
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
    pub(crate) async fn company_facts(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        company: &mut CustomerCompany,
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
    pub(crate) async fn frozen_party(
        &self,
        client: &(impl GenericClient + Sync),
        scope: &Scope,
        id: i64,
    ) -> Result<Value> {
        let (company, masked) =
            bilibi_company::detail(client, scope, id)
                .await
                .map_err(|error| match error {
                    Error::Forbidden => invalid(
                        "customer_id",
                        "needs Companies view permission to freeze the company's details",
                    ),
                    Error::NotFound => invalid(
                        "customer_id",
                        "is linked to a company that no longer exists",
                    ),
                    other => other.into(),
                })?;
        if let Some((_, label)) = FACTS.iter().find(|(field, _)| masked.contains(*field)) {
            return Err(invalid(
                "customer_id",
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

    /// Whether a new invoice may name this customer: linked, available for
    /// Finance, and its company active in Core. A pending company has not
    /// been activated yet, a suspended or archived one is not open for new
    /// business; receipts and credit notes stay possible.
    pub(crate) fn invoiceable(customer: &Customer) -> Result<()> {
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
            "active" => Ok(()),
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
    ) -> Result<(i64, CustomerCompany)> {
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
