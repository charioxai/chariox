use super::{AppCatalog, CatalogError, Message, Result, Transaction, ValidatedToolCall, Value};
use std::sync::Arc;

/// The same immutable, one-use validation correlation, retaining its catalog
/// across writer/worker awaits. It carries no binding or operation permission.
pub struct OwnedValidatedToolCall {
    catalog: Arc<AppCatalog>,
    name: String,
    request: Message,
}

impl ValidatedToolCall<'_> {
    pub fn into_owned(self, catalog: Arc<AppCatalog>) -> Result<OwnedValidatedToolCall> {
        if !std::ptr::eq(self.catalog, Arc::as_ptr(&catalog)) {
            return Err(CatalogError::Provenance);
        }
        Ok(OwnedValidatedToolCall {
            catalog,
            name: self.name,
            request: self.request,
        })
    }
}

impl OwnedValidatedToolCall {
    pub fn catalog(&self) -> &Arc<AppCatalog> {
        &self.catalog
    }
    pub fn request(&self) -> &Message {
        &self.request
    }
    pub fn accept(
        self,
        tx: &Transaction<'_>,
        owner: &str,
        response: Message,
        now_ms: u64,
    ) -> Result<Value> {
        ValidatedToolCall {
            catalog: &self.catalog,
            name: self.name,
            request: self.request,
        }
        .accept(tx, owner, response, now_ms)
    }
}
