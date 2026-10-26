//! The shared release path accepts only proofs created by either verifier.
//! Version-specific signing and runtime contracts stay in their own modules.
use crate::{
    v2, Capabilities, Declarations, Manifest, Migrations, TrustedPublisher, Ui, VerifiedPackage,
};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::VerifiedPackage<'_> {}
    impl Sealed for super::v2::VerifiedPackageV2<'_> {}
    impl Sealed for super::VerifiedAppPackage<'_> {}
}

pub struct ManifestRef<'a> {
    pub app_id: &'a String,
    pub version: &'a String,
    pub capabilities: &'a Capabilities,
    pub ui: &'a Ui,
    pub migrations: &'a Option<Migrations>,
}

#[derive(Clone)]
pub enum ReleaseManifest {
    Node(Manifest),
    Wasm(v2::ManifestV2),
}
impl ReleaseManifest {
    pub fn node(&self) -> Option<&Manifest> {
        match self {
            Self::Node(manifest) => Some(manifest),
            Self::Wasm(_) => None,
        }
    }
    pub fn wasm(&self) -> Option<&v2::ManifestV2> {
        match self {
            Self::Wasm(manifest) => Some(manifest),
            Self::Node(_) => None,
        }
    }
}

pub trait VerifiedApp: sealed::Sealed {
    fn manifest(&self) -> ManifestRef<'_>;
    fn release_manifest(&self) -> ReleaseManifest;
    fn signer(&self) -> &TrustedPublisher;
    fn declarations(&self) -> &Declarations;
    fn package_digest(&self) -> &str;
    fn files(&self) -> impl Iterator<Item = (&str, &[u8])>;
    fn ui_actions(&self) -> Vec<v2::UiActionDeclaration>;
}

macro_rules! proof {
    ($ty:ty, $variant:ident, $ui_actions:expr) => {
        impl VerifiedApp for $ty {
            fn manifest(&self) -> ManifestRef<'_> {
                let manifest = self.manifest();
                ManifestRef {
                    app_id: &manifest.app_id,
                    version: &manifest.version,
                    capabilities: &manifest.capabilities,
                    ui: &manifest.ui,
                    migrations: &manifest.migrations,
                }
            }
            fn release_manifest(&self) -> ReleaseManifest {
                ReleaseManifest::$variant(self.manifest().clone())
            }
            fn signer(&self) -> &TrustedPublisher {
                self.signer()
            }
            fn declarations(&self) -> &Declarations {
                self.declarations()
            }
            fn package_digest(&self) -> &str {
                self.package_digest()
            }
            fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
                self.files()
            }
            fn ui_actions(&self) -> Vec<v2::UiActionDeclaration> {
                ($ui_actions)(self)
            }
        }
    };
}
proof!(VerifiedPackage<'_>, Node, |_: &VerifiedPackage<'_>| {
    Vec::new()
});
proof!(
    v2::VerifiedPackageV2<'_>,
    Wasm,
    |package: &v2::VerifiedPackageV2<'_>| package.ui_actions().to_vec()
);

#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Both variants are bounded preparation proofs; keep their similarly sized manifests inline"
)]
pub enum VerifiedAppPackage<'a> {
    Node(VerifiedPackage<'a>),
    Wasm(v2::VerifiedPackageV2<'a>),
}
impl VerifiedApp for VerifiedAppPackage<'_> {
    fn ui_actions(&self) -> Vec<v2::UiActionDeclaration> {
        match self {
            Self::Node(_) => Vec::new(),
            Self::Wasm(package) => package.ui_actions().to_vec(),
        }
    }
    fn manifest(&self) -> ManifestRef<'_> {
        match self {
            Self::Node(package) => VerifiedApp::manifest(package),
            Self::Wasm(package) => VerifiedApp::manifest(package),
        }
    }
    fn release_manifest(&self) -> ReleaseManifest {
        match self {
            Self::Node(package) => package.release_manifest(),
            Self::Wasm(package) => package.release_manifest(),
        }
    }
    fn signer(&self) -> &TrustedPublisher {
        match self {
            Self::Node(package) => package.signer(),
            Self::Wasm(package) => package.signer(),
        }
    }
    fn declarations(&self) -> &Declarations {
        match self {
            Self::Node(package) => package.declarations(),
            Self::Wasm(package) => package.declarations(),
        }
    }
    fn package_digest(&self) -> &str {
        match self {
            Self::Node(package) => package.package_digest(),
            Self::Wasm(package) => &package.digests().delivered,
        }
    }
    fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        let files: Box<dyn Iterator<Item = (&str, &[u8])> + '_> = match self {
            Self::Node(package) => Box::new(package.files()),
            Self::Wasm(package) => Box::new(package.files()),
        };
        files
    }
}

/// A bounded claim for selecting enrolled trust, never an executable proof.
pub fn inspect_publisher(bytes: &[u8], limits: &crate::Limits) -> crate::Result<crate::Publisher> {
    let entries = crate::archive::parse(bytes, limits)?;
    let value: serde_json::Value = crate::json::read_json(
        entries[0].data,
        limits.max_metadata_bytes,
        limits,
        crate::ErrorCode::InvalidManifest,
        true,
    )?;
    match value["schema"].as_str() {
        Some(v2::APP_SCHEMA_V2) => {
            let manifest: v2::ManifestV2 = serde_json::from_value(value).map_err(|_| {
                crate::PackageError::new(
                    crate::ErrorCode::InvalidManifest,
                    "manifest does not match chariox.app.v2",
                )
            })?;
            manifest.validate(limits)?;
            Ok(manifest.publisher)
        }
        Some(crate::APP_SCHEMA) => Ok(crate::inspect_untrusted(bytes, limits)?.manifest.publisher),
        _ => Err(crate::PackageError::new(
            crate::ErrorCode::IncompatibleContract,
            "unsupported App package schema; rebuild as chariox.app.v2",
        )),
    }
}

/// Restart reads retain v1 while P1 ports the installed Apps. New v2 installs
/// and restarts use exactly the same publisher policy and signature verifier.
pub fn verify_app<'a>(
    bytes: &'a [u8],
    policy: &crate::VerificationPolicy,
) -> crate::Result<VerifiedAppPackage<'a>> {
    let entries = crate::archive::parse(bytes, &policy.limits)?;
    let value: serde_json::Value = crate::json::read_json(
        entries[0].data,
        policy.limits.max_metadata_bytes,
        &policy.limits,
        crate::ErrorCode::InvalidManifest,
        true,
    )?;
    if value["schema"] == v2::APP_SCHEMA_V2 {
        let mut v2_policy = v2::PolicyV2::new(
            [
                v2::RequiredFeature::WasmComponent,
                v2::RequiredFeature::AgentPanel,
                v2::RequiredFeature::Connections,
                v2::RequiredFeature::DirectedEvents,
            ]
            .into(),
            vec![v2::SupportedWorld {
                name: "chariox:app/app".into(),
                major: 1,
                minor: 0,
            }],
            policy.trusted_publishers.clone(),
        );
        v2_policy.limits = policy.limits.clone();
        v2_policy.resource_policies = policy.resource_policies.clone();
        v2::verify(bytes, &v2_policy).map(VerifiedAppPackage::Wasm)
    } else {
        crate::verify(bytes, policy).map(VerifiedAppPackage::Node)
    }
}
