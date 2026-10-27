use std::path::Path;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::fs;
use crate::{
    json, manifest::valid_identifier, ErrorCode, Limits, PackageError, Publisher, Result,
    TrustedPublisher,
};

const TRUST_SCHEMA: &str = "chariox.developer-publisher.v1";

/// Explicit enrollment material. Loading it for a developer validation command
/// does not enroll it into the kernel or make packages trusted globally.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublisherFile {
    pub schema: String,
    pub publisher: Publisher,
    pub algorithm: String,
    pub public_key: String,
}

impl PublisherFile {
    pub fn trusted_publisher(&self) -> Result<TrustedPublisher> {
        let invalid = || {
            PackageError::new(
                ErrorCode::InvalidDeveloperKey,
                "invalid developer publisher enrollment file",
            )
        };
        let bytes = BASE64.decode(&self.public_key).map_err(|_| invalid())?;
        let array: [u8; 32] = bytes.try_into().map_err(|_| invalid())?;
        let public_key = VerifyingKey::from_bytes(&array).map_err(|_| invalid())?;
        if self.schema != TRUST_SCHEMA
            || self.algorithm != "ed25519"
            || !valid_identifier(&self.publisher.id)
            || self.publisher.name.trim().is_empty()
            || self.publisher.name.len() > 256
            || self.publisher.key_id != key_id(&public_key)
            || BASE64.encode(array) != self.public_key
            || public_key.is_weak()
        {
            return Err(invalid());
        }
        Ok(TrustedPublisher {
            publisher_id: self.publisher.id.clone(),
            key_id: self.publisher.key_id.clone(),
            public_key,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeygenReport {
    pub publisher: Publisher,
    pub fingerprint: String,
    pub private_key_path: String,
    pub enrollment_path: String,
    pub trust_status: &'static str,
}

pub(crate) fn key_id(key: &VerifyingKey) -> String {
    format!("dev-{:x}", Sha256::digest(key.as_bytes()))
}

pub fn keygen(
    publisher_id: &str,
    publisher_name: &str,
    private_path: &Path,
    enrollment_path: &Path,
) -> Result<KeygenReport> {
    if !valid_identifier(publisher_id)
        || publisher_name.trim().is_empty()
        || publisher_name.len() > 256
        || private_path == enrollment_path
    {
        return Err(PackageError::new(
            ErrorCode::InvalidArguments,
            "publisher identity, name, and distinct output paths are required",
        ));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    OsRng
        .try_fill_bytes(seed.as_mut())
        .map_err(|_| fs::io_error())?;
    let signing = SigningKey::from_bytes(&seed);
    let publisher = Publisher {
        id: publisher_id.to_owned(),
        key_id: key_id(&signing.verifying_key()),
        name: publisher_name.to_owned(),
    };
    let enrollment = PublisherFile {
        schema: TRUST_SCHEMA.to_owned(),
        publisher: publisher.clone(),
        algorithm: "ed25519".to_owned(),
        public_key: BASE64.encode(signing.verifying_key().as_bytes()),
    };
    enrollment.trusted_publisher()?;
    // Prepare both before publishing. A crash can leave only a valid private
    // key; enrollment is published last and is never added to kernel trust.
    let private = fs::PreparedFile::new(private_path, seed.as_ref(), true)?;
    let public = fs::PreparedFile::new(enrollment_path, &json::canonical(&enrollment)?, false)?;
    if private.same_destination(&public)? {
        return Err(PackageError::new(
            ErrorCode::InvalidArguments,
            "private and enrollment outputs must differ",
        ));
    }
    private.publish()?;
    public.publish().map_err(|_| {
        PackageError::new(
            ErrorCode::Io,
            "private key was created; enrollment publication failed; existing files were preserved",
        )
    })?;
    Ok(KeygenReport {
        publisher,
        fingerprint: format!(
            "sha256:{:x}",
            Sha256::digest(signing.verifying_key().as_bytes())
        ),
        private_key_path: private_path.to_string_lossy().into_owned(),
        enrollment_path: enrollment_path.to_string_lossy().into_owned(),
        trust_status: "enrollment-material-only",
    })
}

pub fn read_publisher(path: &Path, limits: &Limits) -> Result<PublisherFile> {
    let bytes = fs::read(path, 4096, false)?;
    let value: PublisherFile =
        json::read_json(&bytes, 4096, limits, ErrorCode::InvalidDeveloperKey, false)?;
    value.trusted_publisher()?;
    Ok(value)
}

pub(crate) struct SigningInput {
    pub key: SigningKey,
    pub directory: fs::Directory,
    pub file: std::fs::File,
}

pub(crate) fn read_signing_key(path: &Path) -> Result<SigningInput> {
    let (directory, file) = fs::open_file(path, true)?;
    let bytes = Zeroizing::new(fs::read_open(
        file.try_clone().map_err(|_| fs::io_error())?,
        32,
    )?);
    let seed: &[u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        PackageError::new(
            ErrorCode::InvalidDeveloperKey,
            "private signing key must contain exactly 32 seed bytes",
        )
    })?;
    Ok(SigningInput {
        key: SigningKey::from_bytes(seed),
        directory,
        file,
    })
}
