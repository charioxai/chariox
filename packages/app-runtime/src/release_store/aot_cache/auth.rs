//! An installation-local integrity identity, never publisher/review authority.
//! Only its public half enters the supervisor's private launch configuration.
use super::*;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;

pub(super) struct Auth(SigningKey);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedProvenance {
    provenance: AotProvenance,
    signature: Vec<u8>,
}
fn subject(provenance: &AotProvenance) -> Result<Vec<u8>> {
    let mut bytes = b"chariox.app.aot-cache.v1\n".to_vec();
    bytes.extend(serde_json_canonicalizer::to_vec(provenance).map_err(|_| AotError::Integrity)?);
    Ok(bytes)
}
impl Auth {
    pub(super) fn open(root: &Dir) -> Result<Self> {
        let _lock = lock(root, ".lock", std::time::Duration::from_secs(1))?;
        let name = OsStr::new(".provenance-key");
        match root.open_private_file(name) {
            Ok(mut file) => {
                if file.metadata()?.len() == 32 {
                    let mut seed = [0; 32];
                    file.read_exact(&mut seed)?;
                    return Ok(Self(SigningKey::from_bytes(&seed)));
                }
                // This identity protects only disposable derived code. A
                // damaged seed may be replaced: old signatures then fail, so
                // entries are refused and recompiled from retained components.
            }
            Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut seed = [0; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut seed)
            .map_err(|_| AotError::Integrity)?;
        root.atomic_replace(name, &seed)?;
        Ok(Self(SigningKey::from_bytes(&seed)))
    }
    pub(super) fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub(super) fn sign(&self, provenance: AotProvenance) -> Result<Vec<u8>> {
        let signature = self.0.sign(&subject(&provenance)?).to_bytes().to_vec();
        serde_json_canonicalizer::to_vec(&SignedProvenance {
            provenance,
            signature,
        })
        .map_err(|_| AotError::Integrity)
    }
}
pub(super) fn verify(bytes: &[u8], public_key: &[u8; 32]) -> Result<AotProvenance> {
    let signed: SignedProvenance =
        serde_json::from_slice(bytes).map_err(|_| AotError::Integrity)?;
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| AotError::Integrity)?;
    let signature = Signature::from_slice(&signed.signature).map_err(|_| AotError::Integrity)?;
    key.verify_strict(&subject(&signed.provenance)?, &signature)
        .map_err(|_| AotError::Integrity)?;
    Ok(signed.provenance)
}
