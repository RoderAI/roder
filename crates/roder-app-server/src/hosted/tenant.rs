//! Tenant registry for the hosted gateway.

use std::collections::BTreeMap;
use std::sync::RwLock;

use roder_api::identity::{TenantContext, TenantId};

#[derive(Default)]
pub struct TenantRegistry {
    tenants: RwLock<BTreeMap<TenantId, TenantContext>>,
}

impl TenantRegistry {
    pub fn insert(&self, tenant: TenantContext) {
        self.tenants
            .write()
            .unwrap()
            .insert(tenant.tenant_id.clone(), tenant);
    }

    pub fn get(&self, tenant_id: &str) -> Option<TenantContext> {
        self.tenants.read().unwrap().get(tenant_id).cloned()
    }

    pub fn list(&self) -> Vec<TenantContext> {
        self.tenants.read().unwrap().values().cloned().collect()
    }
}
