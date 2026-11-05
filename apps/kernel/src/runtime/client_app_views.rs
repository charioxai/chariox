//! MP-08/09/10/11: bounded delivery leases, without a worker or managed activity fence.
use crate::{
    durable_state::DurableKernelStateStore,
    local::{
        AppRequestErrorCode as Error, ClientAppViewDescriptor as Descriptor,
        ClientAppViewRequest as Request, ClientAppViewResponse as Response, ClientAppViewTrust,
    },
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chariox_app_package::{VerifiedApp, VerifiedAppPackage};
use chariox_app_runtime::release_store::{ReleaseStore, VerifiedReleaseLease};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHUNK_BYTES: u32 = 64 * 1024;
const LEASE_DURATION: Duration = Duration::from_secs(60);

struct Delivery {
    owner: String,
    descriptor: Descriptor,
    generation: u64,
    entry: String,
    size: usize,
    lease: VerifiedReleaseLease,
    expires: Instant,
    expiration: tokio::task::AbortHandle,
}
impl Drop for Delivery {
    fn drop(&mut self) {
        self.expiration.abort();
    }
}
#[derive(Clone, Default)]
pub(crate) struct ClientAppViews(Arc<Mutex<BTreeMap<String, Arc<Delivery>>>>);

impl ClientAppViews {
    pub(crate) fn handle(
        &self,
        store: &DurableKernelStateStore,
        owner: &str,
        request: Request,
    ) -> Result<Response, Error> {
        if let Request::Open { installation_id } = request {
            return self.open(store, owner, &installation_id);
        }
        let id = match &request {
            Request::Read { view_id, .. }
            | Request::Check { view_id }
            | Request::Close { view_id } => view_id,
            Request::Open { .. } => unreachable!(),
        };
        let view = self
            .0
            .lock()
            .map_err(|_| Error::StorageUnavailable)?
            .get(id)
            .filter(|view| view.owner == owner)
            .cloned()
            .ok_or(Error::NotFound)?;
        if matches!(request, Request::Close { .. }) {
            self.remove(id);
            return Ok(Response::Closed {
                view_id: id.clone(),
            });
        }
        self.check(store, owner, id, &view)?;
        match request {
            Request::Read {
                view_id,
                offset,
                max_bytes,
            } => {
                if max_bytes == 0 || max_bytes > MAX_CHUNK_BYTES || offset as usize >= view.size {
                    return Err(Error::InvalidRequest);
                }
                let bytes = view
                    .lease
                    .read_ui_asset(&view.entry, MAX_ENTRY_BYTES)
                    .map_err(|_| Error::Conflict)?;
                let end = (offset as usize + max_bytes as usize).min(bytes.len());
                let body_base64 = BASE64.encode(&bytes[offset as usize..end]);
                self.check(store, owner, &view_id, &view)?;
                Ok(Response::Chunk {
                    view_id,
                    offset,
                    body_base64,
                    eof: end == view.size,
                })
            }
            Request::Check { view_id } => Ok(Response::Valid {
                view_id,
                generation: view.descriptor.generation.clone(),
            }),
            _ => unreachable!(),
        }
    }

    fn check(
        &self,
        store: &DurableKernelStateStore,
        owner: &str,
        id: &str,
        view: &Delivery,
    ) -> Result<(), Error> {
        let current =
            store.check_app_binding_in_room(owner, &view.descriptor.installation_id, None);
        let present = self
            .0
            .lock()
            .map_err(|_| Error::StorageUnavailable)?
            .contains_key(id);
        if view.expires <= Instant::now()
            || !current.is_ok_and(|active| {
                active.generation == view.generation
                    && active.release.package_digest == view.descriptor.package_digest
            })
            || !present
        {
            self.remove(id);
            return Err(Error::Conflict);
        }
        Ok(())
    }

    fn remove(&self, id: &str) {
        if let Ok(mut views) = self.0.lock() {
            views.remove(id);
        }
    }

    fn open(
        &self,
        store: &DurableKernelStateStore,
        owner: &str,
        installation: &str,
    ) -> Result<Response, Error> {
        if installation.is_empty() || installation.len() > 128 {
            return Err(Error::InvalidRequest);
        }
        let release = store
            .active_app_release(owner, installation)
            .map_err(Error::from)?;
        let verified = release.verify().map_err(Error::from)?;
        let VerifiedAppPackage::Wasm(package) = &verified else {
            return Err(Error::InvalidRequest);
        };
        let entry = package
            .manifest()
            .ui
            .entry
            .strip_prefix("ui/")
            .ok_or(Error::InvalidRequest)?
            .to_owned();
        let size = package
            .file(&package.manifest().ui.entry)
            .ok_or(Error::Conflict)?
            .len();
        if size == 0 || size > MAX_ENTRY_BYTES {
            return Err(Error::LimitExceeded);
        }
        let lease = ReleaseStore::open_or_create(store.path())
            .and_then(|store| store.lease_verified(&verified, release.bytes()))
            .map_err(|_| Error::Conflict)?;
        let id = format!("client-app-{:032x}", rand::random::<u128>());
        let descriptor = Descriptor {
            view_id: id.clone(),
            installation_id: installation.into(),
            generation: release.generation().to_string(),
            package_digest: verified.package_digest().into(),
            bundle: package.view_controls().clone(),
            trust: ClientAppViewTrust {
                publisher_id: package.signer().publisher_id.clone(),
                key_id: package.signer().key_id.clone(),
                content_digest: package.digests().content.clone(),
                public_key: BASE64.encode(package.signer().public_key.as_bytes()),
            },
        };
        let expired_id = id.clone();
        let weak = Arc::downgrade(&self.0);
        let expiration = tokio::spawn(async move {
            tokio::time::sleep(LEASE_DURATION).await;
            if let Some(views) = weak.upgrade() {
                if let Ok(mut views) = views.lock() {
                    views.remove(&expired_id);
                }
            }
        });
        let view = Arc::new(Delivery {
            owner: owner.into(),
            descriptor: descriptor.clone(),
            generation: release.generation(),
            entry,
            size,
            lease,
            expires: Instant::now() + LEASE_DURATION,
            expiration: expiration.abort_handle(),
        });
        {
            let mut views = self.0.lock().map_err(|_| Error::StorageUnavailable)?;
            views.retain(|_, view| view.expires > Instant::now());
            if views.len() >= 128 || views.values().filter(|view| view.owner == owner).count() >= 32
            {
                return Err(Error::LimitExceeded);
            }
            views.insert(id.clone(), view.clone());
        }
        self.check(store, owner, &id, &view)?;
        Ok(Response::Opened(Box::new(descriptor)))
    }
}

#[cfg(test)]
mod tests;
