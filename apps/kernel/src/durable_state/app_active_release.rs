//! The active release of an installation, re-read from the release store and
//! re-verified against the owner's current publisher trust. A revoked
//! publisher or inactive installation yields nothing. No worker is needed.
use super::DurableKernelStateStore;
use chariox_app_package::{verify, VerificationPolicy, VerifiedPackage};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    app_inbox::IncomingCatalog,
    app_outbox::EventCatalog,
    installation::{InstallationRegistry, StageTrustBinding},
    publisher_trust::{PublisherTrustRegistry, TrustedPublisherSnapshot},
    release_store::ReleaseStore,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveReleaseError {
    NotActive,
    Untrusted,
    Unavailable,
    Invalid,
    Storage,
}

pub(crate) struct ActiveRelease {
    bytes: Vec<u8>,
    binding: StageTrustBinding,
    trust: TrustedPublisherSnapshot,
}

impl ActiveRelease {
    /// Reads the stored archive of an already-resolved binding. Worker start
    /// and worker-free reads share this and `verify`, so their policy matches.
    pub(crate) fn load(
        store_path: &std::path::Path,
        binding: StageTrustBinding,
        trust: TrustedPublisherSnapshot,
    ) -> Result<Self, ActiveReleaseError> {
        let bytes = ReleaseStore::open_or_create(store_path)
            .and_then(|store| store.open_stored_archive(binding.package_digest()))
            .and_then(|mut archive| archive.read_bytes())
            .map_err(|_| ActiveReleaseError::Unavailable)?;
        Ok(Self {
            bytes,
            binding,
            trust,
        })
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn generation(&self) -> u64 {
        self.binding.token().generation
    }
    pub(crate) fn event_catalog(
        &self,
        verified: &VerifiedPackage<'_>,
    ) -> Result<Arc<EventCatalog>, ActiveReleaseError> {
        let catalog = AppCatalog::compile(verified, &self.binding, &self.trust)
            .map_err(|_| ActiveReleaseError::Untrusted)?;
        EventCatalog::compile(verified, Arc::new(catalog))
            .map(Arc::new)
            .map_err(|_| ActiveReleaseError::Invalid)
    }
    pub(crate) fn verify(&self) -> Result<VerifiedPackage<'_>, ActiveReleaseError> {
        let verified = verify(
            &self.bytes,
            &VerificationPolicy::new(
                crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                vec![self.trust.publisher().clone()],
            ),
        )
        .map_err(|_| ActiveReleaseError::Invalid)?;
        if verified.package_digest() != self.binding.package_digest() {
            return Err(ActiveReleaseError::Invalid);
        }
        Ok(verified)
    }
}

impl DurableKernelStateStore {
    pub(crate) fn active_app_release(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<ActiveRelease, ActiveReleaseError> {
        let (binding, trust) = {
            let mut connection = self
                .lock_connection("durable_state.active_app_release")
                .map_err(|_| ActiveReleaseError::Storage)?;
            let binding = InstallationRegistry::new(&mut connection)
                .active_trust(owner, installation)
                .map_err(|_| ActiveReleaseError::NotActive)?;
            let trust = PublisherTrustRegistry::new(&mut connection)
                .trusted_publisher(owner, binding.publisher_id(), binding.key_id())
                .map_err(|_| ActiveReleaseError::Untrusted)?;
            (binding, trust)
        };
        ActiveRelease::load(self.path(), binding, trust)
    }

    /// The installation's verified event catalog, whether or not it runs.
    pub(crate) fn active_app_event_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Arc<EventCatalog>, ActiveReleaseError> {
        let release = self.active_app_release(owner, installation)?;
        let verified = release.verify()?;
        release.event_catalog(&verified)
    }

    /// The active generation and its signed incoming event schemas.
    pub(crate) fn active_app_incoming_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(u64, IncomingCatalog), ActiveReleaseError> {
        let release = self.active_app_release(owner, installation)?;
        let verified = release.verify()?;
        let catalog =
            IncomingCatalog::compile(&verified).map_err(|_| ActiveReleaseError::Invalid)?;
        Ok((release.generation(), catalog))
    }
}
