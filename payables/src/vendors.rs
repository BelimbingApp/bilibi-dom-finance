//! Vendor master and permanent historical supplier-key confirmations. Core
//! owns identity; mappings never update a bill, payment, or aged balance.
use crate::*;
use crate::{
    settings::{account, string},
    store::Table,
};
use bilibi_finance_ledger::{AccountType, CompanyFacts, CompanyLinks};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vendor {
    pub id: i64,
    pub code: String,
    pub record_kind: String,
    pub record_id: String,
    pub payment_terms_days: i32,
    pub payable_account_id: Option<i64>,
    pub expense_account_id: Option<i64>,
    pub tax_code_id: Option<i64>,
    pub active: bool,
    pub company: CompanyFacts,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VendorInput {
    pub code: String,
    pub core_company_id: i64,
    pub payment_terms_days: i32,
    pub payable_account_id: Option<i64>,
    pub expense_account_id: Option<i64>,
    pub tax_code_id: Option<i64>,
    pub active: bool,
}
/// One unchanged key and every distinct snapshot it has carried. Collisions
/// and changed facts are evidence for a human decision, never an auto-match.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SupplierHistory {
    pub supplier_key: String,
    pub vendor_id: Option<i64>,
    pub snapshots: Value,
    pub documents: i64,
    pub collision_keys: Vec<String>,
}
pub(crate) async fn mapped(
    c: &dyn OperationClient,
    tenant: i64,
    company: i64,
    key: &str,
) -> bilibi_database::Result<Option<i64>> {
    Ok(c.query_opt("SELECT vendor_id FROM finance_payables_vendor_keys WHERE tenant_id=$1 AND company_id=$2 AND supplier_key=$3", &[&tenant,&company,&key]).await?.map(|r|r.get(0)))
}
pub(crate) fn key(id: i64) -> String {
    format!("vendor:{id}")
}
impl Payables {
    pub(crate) async fn vendor_on(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        id: i64,
    ) -> Result<Vendor> {
        let row = c.query_opt("SELECT to_jsonb(v) FROM finance_payables_vendors v WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&s.tenant(), &company,&id]).await?.ok_or_else(|| invalid("vendor_id","is not a vendor of this company"))?;
        let mut value: Value = row.get(0);
        let core_id = value["record_id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .ok_or(Error::Invalid)?;
        value["company"] = json!(
            CompanyLinks::new(&self.parties)
                .company_summary(c, s, core_id)
                .await?
        );
        serde_json::from_value(value).map_err(|_| Error::Invalid.into())
    }
    pub async fn get_vendor(&self, s: &Scope, company: i64, id: i64) -> Result<Vendor> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        let mut v = self.vendor_on(&**c, s, company, id).await?;
        CompanyLinks::new(&self.parties)
            .company_facts(&**c, s, &mut v.company)
            .await?;
        Ok(v)
    }
    pub async fn list_vendors(
        &self,
        s: &Scope,
        company: i64,
        after: i64,
        limit: i64,
    ) -> Result<Vec<Vendor>> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        let mut records = self.vendors_on(&**c, s, company, after, limit).await?;
        for v in &mut records {
            CompanyLinks::new(&self.parties)
                .company_facts(&**c, s, &mut v.company)
                .await?;
        }
        Ok(records)
    }
    pub(crate) async fn vendors_on(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        after: i64,
        limit: i64,
    ) -> Result<Vec<Vendor>> {
        if after < 0 || !(1..=500).contains(&limit) {
            return Err(PayablesError::InvalidOption);
        }
        let mut records = Vec::new();
        for row in c.query("SELECT id FROM finance_payables_vendors WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4",&[&s.tenant(),&company,&after,&limit]).await? { records.push(self.vendor_on(c,s,company,row.get(0)).await?); }
        Ok(records)
    }
    pub(crate) async fn prepare_vendor(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        id: Option<i64>,
        input: &VendorInput,
    ) -> Result<Value> {
        let code = string(&input.code, "code", 50, true)?.ok_or(PayablesError::InvalidOption)?;
        if !(0..=3650).contains(&input.payment_terms_days) {
            return Err(invalid("payment_terms_days", "must be between 0 and 3650"));
        }
        if let Some(id) = id {
            let old = self.vendor_on(c, s, company, id).await?;
            if old.code != code || old.record_id != input.core_company_id.to_string() {
                return Err(invalid(
                    "vendor_id",
                    "code and linked company cannot change",
                ));
            }
        } else {
            if input.core_company_id == company {
                return Err(invalid("core_company_id", "cannot be the company itself"));
            }
            self.records
                .check(c, s, "company", &input.core_company_id.to_string())
                .await?
                .map_err(|e| invalid("core_company_id", e.message()))?;
            let summary = CompanyLinks::new(&self.parties)
                .company_summary(c, s, input.core_company_id)
                .await?;
            if summary.status == "archived" {
                return Err(invalid("core_company_id", "is an archived company"));
            }
            if summary.status == "unavailable" {
                return Err(invalid("core_company_id", "does not exist"));
            }
            if c.query_opt("SELECT id FROM finance_payables_vendors WHERE tenant_id=$1 AND company_id=$2 AND code=$3", &[&s.tenant(),&company,&code]).await?.is_some() { return Err(invalid("code","has already been taken")); }
            if c.query_opt("SELECT id FROM finance_payables_vendors WHERE tenant_id=$1 AND company_id=$2 AND record_id=$3", &[&s.tenant(),&company,&input.core_company_id.to_string()]).await?.is_some() { return Err(invalid("core_company_id","is already a vendor of this company")); }
        }
        let accounts = self.ledger.list_accounts_on(c, s, company).await?;
        if let Some(id) = input.payable_account_id {
            account(
                &accounts,
                id,
                "payable_account_id",
                &[AccountType::Liability],
            )?;
        }
        if let Some(id) = input.expense_account_id {
            account(
                &accounts,
                id,
                "expense_account_id",
                &[AccountType::Expense, AccountType::Asset],
            )?;
        }
        if let Some(id) = input.tax_code_id {
            if !store::taxes(c, s.tenant(), company)
                .await?
                .iter()
                .any(|t| t.id == id && t.active)
            {
                return Err(invalid(
                    "tax_code_id",
                    "is not an active tax code of this company",
                ));
            }
        }
        Ok(
            json!({"tenant_id":s.tenant(),"company_id":company,"code":code,"record_kind":"company","record_id":input.core_company_id.to_string(),"payment_terms_days":input.payment_terms_days,"payable_account_id":input.payable_account_id,"expense_account_id":input.expense_account_id,"tax_code_id":input.tax_code_id,"active":input.active}),
        )
    }
    pub async fn create_vendor(
        &self,
        s: &Scope,
        company: i64,
        input: &VendorInput,
    ) -> Result<Vendor> {
        self.save_vendor(s, company, None, input).await
    }
    pub async fn update_vendor(
        &self,
        s: &Scope,
        company: i64,
        id: i64,
        input: &VendorInput,
    ) -> Result<Vendor> {
        self.save_vendor(s, company, Some(id), input).await
    }
    async fn save_vendor(
        &self,
        s: &Scope,
        company: i64,
        id: Option<i64>,
        input: &VendorInput,
    ) -> Result<Vendor> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                MANAGE_SETTINGS,
                "finance.payables.vendor.save",
            )
            .await?;
        let v = self.save_vendor_on(&mut tx, s, company, id, input).await?;
        tx.commit().await?;
        Ok(v)
    }
    pub(crate) async fn save_vendor_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        id: Option<i64>,
        input: &VendorInput,
    ) -> Result<Vendor> {
        join(tx, s, company, MANAGE_SETTINGS).await?;
        let mut value = self.prepare_vendor(tx.sql(), s, company, id, input).await?;
        let reserved = if id.is_none() {
            let reserved: i64 = tx
                .sql()
                .query_one(
                    "SELECT nextval(pg_get_serial_sequence('finance_payables_vendors','id'))",
                    &[],
                )
                .await?
                .get(0);
            let key = key(reserved);
            if tx.sql().query_opt("SELECT id FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2 AND party_key=$3 LIMIT 1",&[&s.tenant(),&company,&key]).await?.is_some() {
                return Err(invalid("code","the new vendor key collides with historical evidence; retry to allocate another identifier"));
            }
            value["id"] = json!(reserved);
            Some(reserved)
        } else {
            None
        };
        let tenant = s.tenant();
        let actor = s.user();
        let imp = s.impersonator().map(|p| p.user());
        let id=tx.mutate(move |sql| Box::pin(async move {
            if let Some(id)=id {
                let before=store::raw(sql,Table::Vendors,tenant,company,id).await?;
                sql.execute("UPDATE finance_payables_vendors SET payment_terms_days=($4::jsonb->>'payment_terms_days')::int,payable_account_id=($4::jsonb->>'payable_account_id')::bigint,expense_account_id=($4::jsonb->>'expense_account_id')::bigint,tax_code_id=($4::jsonb->>'tax_code_id')::bigint,active=($4::jsonb->>'active')::boolean,updated_at=timezone('UTC',clock_timestamp()) WHERE tenant_id=$1 AND company_id=$2 AND id=$3", &[&tenant,&company,&id,&value]).await?;
                let after=store::raw(sql,Table::Vendors,tenant,company,id).await?;
                Ok((id,vec![store::update_change(Table::Vendors,id,before,after)]))
            } else {
                let id=reserved.ok_or(Error::Invalid)?;
                let row=sql.query_one("INSERT INTO finance_payables_vendors SELECT (jsonb_populate_record(NULL::finance_payables_vendors,$1::jsonb || jsonb_build_object('inserted_at',timezone('UTC',clock_timestamp()),'updated_at',timezone('UTC',clock_timestamp())))).* RETURNING to_jsonb(finance_payables_vendors)",&[&value]).await?;
                let change=bilibi_database::Change{kind:"finance.payables.vendor".into(),id,old:Value::Null,new:row.get(0)};
                let mapping=json!({"tenant_id":tenant,"company_id":company,"vendor_id":id,"supplier_key":key(id),"reason":"Vendor created","confirmed_by_user_id":actor,"confirmed_by_impersonator_id":imp});
                let (_,mapped)=store::insert(sql,Table::VendorKeys,mapping).await?;
                Ok((id,vec![change,mapped]))
            }
        })).await?;
        self.vendor_on(tx.sql(), s, company, id).await
    }
    pub(crate) fn billable(v: &Vendor) -> Result<()> {
        if !v.active {
            return Err(invalid("vendor_id", "is an inactive vendor"));
        }
        match v.company.status.as_str() {
            "active" => Ok(()),
            "pending" => Err(invalid(
                "vendor_id",
                "is a pending company; activate it in Companies before recording a bill",
            )),
            "suspended" => Err(invalid(
                "vendor_id",
                "is a suspended company; reactivate it in Companies before recording a bill",
            )),
            "archived" => Err(invalid("vendor_id", "is an archived company")),
            _ => Err(invalid(
                "vendor_id",
                "is linked to a company that is not available",
            )),
        }
    }
    pub async fn supplier_history(&self, s: &Scope, company: i64) -> Result<Vec<SupplierHistory>> {
        let c = self.db.get().await.map_err(Error::from)?;
        authorize(&**c, s, company, VIEW).await?;
        self.supplier_history_on(&**c, s, company).await
    }
    pub(crate) async fn supplier_history_on(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
    ) -> Result<Vec<SupplierHistory>> {
        let rows=c.query("WITH facts AS (SELECT DISTINCT party_key,party_name,party_tax_id,party_registration_id,party_address FROM finance_payables_documents WHERE tenant_id=$1 AND company_id=$2) SELECT f.party_key,(SELECT vendor_id FROM finance_payables_vendor_keys k WHERE k.tenant_id=$1 AND k.company_id=$2 AND k.supplier_key=f.party_key),jsonb_agg(jsonb_build_object('name',party_name,'tax_id',party_tax_id,'registration_id',party_registration_id,'address',party_address) ORDER BY party_name,party_tax_id,party_registration_id,party_address),(SELECT count(*) FROM finance_payables_documents d WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.party_key=f.party_key),ARRAY(SELECT DISTINCT g.party_key FROM facts g WHERE g.party_key<>f.party_key AND (lower(g.party_name) IN (SELECT lower(h.party_name) FROM facts h WHERE h.party_key=f.party_key) OR g.party_tax_id IN (SELECT h.party_tax_id FROM facts h WHERE h.party_key=f.party_key AND h.party_tax_id IS NOT NULL)) ORDER BY g.party_key) FROM facts f GROUP BY f.party_key ORDER BY f.party_key", &[&s.tenant(),&company]).await?;
        Ok(rows
            .into_iter()
            .map(|r| SupplierHistory {
                supplier_key: r.get(0),
                vendor_id: r.get(1),
                snapshots: r.get(2),
                documents: r.get(3),
                collision_keys: r.get(4),
            })
            .collect())
    }
    pub(crate) async fn prepare_mapping(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        company: i64,
        vendor: i64,
        key: &str,
        reason: &str,
    ) -> Result<Value> {
        let v = self.vendor_on(c, s, company, vendor).await?;
        let key = string(key, "supplier_key", 100, true)?.ok_or(PayablesError::InvalidOption)?;
        let reason = string(reason, "reason", 500, true)?.ok_or(PayablesError::InvalidOption)?;
        if mapped(c, s.tenant(), company, &key).await?.is_some() {
            return Err(invalid(
                "supplier_key",
                "already has a confirmed vendor; it cannot be remapped",
            ));
        }
        let history = self
            .supplier_history_on(c, s, company)
            .await?
            .into_iter()
            .find(|h| h.supplier_key == key)
            .ok_or_else(|| invalid("supplier_key", "has no historical documents to review"))?;
        Ok(json!({"vendor":v,"history":history,"reason":reason}))
    }
    pub async fn map_supplier_key(
        &self,
        s: &Scope,
        company: i64,
        vendor: i64,
        key: &str,
        reason: &str,
    ) -> Result<()> {
        let mut c = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut c,
                s,
                company,
                MANAGE_SETTINGS,
                "finance.payables.vendor.map_key",
            )
            .await?;
        self.map_supplier_key_on(&mut tx, s, company, vendor, key, reason)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn map_supplier_key_on(
        &self,
        tx: &mut MutationTransaction<'_>,
        s: &Scope,
        company: i64,
        vendor: i64,
        key: &str,
        reason: &str,
    ) -> Result<()> {
        join(tx, s, company, MANAGE_SETTINGS).await?;
        let review = self
            .prepare_mapping(tx.sql(), s, company, vendor, key, reason)
            .await?;
        let value = json!({"tenant_id":s.tenant(),"company_id":company,"vendor_id":vendor,"supplier_key":key.trim(),"reason":review["reason"],"confirmed_by_user_id":s.user(),"confirmed_by_impersonator_id":s.impersonator().map(|p|p.user())});
        tx.mutate(move |sql| {
            Box::pin(async move {
                let (_, change) = store::insert(sql, Table::VendorKeys, value).await?;
                Ok(((), vec![change]))
            })
        })
        .await?;
        Ok(())
    }
}
