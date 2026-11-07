//! The kernel's credential at the event gateways: its relay identity key.
//!
//! AEDS gets a signed hello for a fresh challenge; every AEGS management request carries a
//! signature over its method, path, body and a single-use nonce. No token is issued or stored.

use chariox_event_protocol::kernel_identity::{
    aeds_hello_message, kernel_id_for_public_key, sign, ManagementRequest, KERNEL_KEY_HEADER,
    KERNEL_NONCE_HEADER, KERNEL_SIGNATURE_HEADER, KERNEL_TIMESTAMP_HEADER, OWNER_ID_HEADER,
};
use chariox_event_protocol::{KernelEnvironmentResume, KernelToAedsMessage};
use rand::RngCore;

#[derive(Clone)]
pub(crate) struct GatewayIdentity {
    private_key: String,
    public_key: String,
}

impl std::fmt::Debug for GatewayIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayIdentity")
            .field("kernel_id", &self.kernel_id())
            .finish_non_exhaustive()
    }
}

impl GatewayIdentity {
    pub(crate) fn from_config(config: &crate::config::DaemonConfig) -> Self {
        Self {
            private_key: config.relay_private_key.clone(),
            public_key: config.relay_public_key.clone(),
        }
    }

    /// The fingerprint of the public key: the id both gateways know this kernel by.
    pub(crate) fn kernel_id(&self) -> String {
        kernel_id_for_public_key(&self.public_key)
    }

    pub(crate) fn hello(
        &self,
        protocol_version: u32,
        nonce: &str,
        environments: Vec<KernelEnvironmentResume>,
    ) -> Result<KernelToAedsMessage, String> {
        let kernel_id = self.kernel_id();
        let signature = sign(&self.private_key, &aeds_hello_message(nonce, &kernel_id))
            .map_err(|error| error.to_string())?;
        Ok(KernelToAedsMessage::Hello {
            protocol_version,
            kernel_id,
            public_key: self.public_key.clone(),
            signature,
            environments,
        })
    }

    /// The five headers that authenticate one management request and name its owner.
    pub(crate) fn management_headers(
        &self,
        method: &str,
        path: &str,
        generator_id: &str,
        owner_id: &str,
        body: &[u8],
    ) -> Result<[(&'static str, String); 5], String> {
        let mut nonce = [0_u8; 16];
        rand::thread_rng().fill_bytes(&mut nonce);
        let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let timestamp_ms = crate::session::unix_epoch_ms();
        let message = ManagementRequest {
            method,
            path,
            generator_id,
            owner_id,
            timestamp_ms,
            nonce: &nonce,
            body,
        }
        .message();
        let signature = sign(&self.private_key, &message).map_err(|error| error.to_string())?;
        Ok([
            (KERNEL_KEY_HEADER, self.public_key.clone()),
            (KERNEL_TIMESTAMP_HEADER, timestamp_ms.to_string()),
            (KERNEL_NONCE_HEADER, nonce),
            (KERNEL_SIGNATURE_HEADER, signature),
            (OWNER_ID_HEADER, owner_id.to_string()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_event_protocol::kernel_identity::verify;

    fn identity() -> GatewayIdentity {
        GatewayIdentity::from_config(&crate::config::DaemonConfig::for_tests())
    }

    #[test]
    fn the_kernel_id_is_the_public_key_thumbprint_the_kernel_already_reports() {
        let identity = identity();
        assert_eq!(
            identity.kernel_id(),
            crate::runtime::terminal_pairings::public_key_thumbprint(&identity.public_key)
        );
    }

    #[test]
    fn the_hello_signature_verifies_for_its_challenge_only() {
        let identity = identity();
        let KernelToAedsMessage::Hello {
            kernel_id,
            public_key,
            signature,
            ..
        } = identity.hello(4, "nonce-1", Vec::new()).unwrap()
        else {
            panic!("a hello was built");
        };
        assert_eq!(
            verify(
                &public_key,
                &aeds_hello_message("nonce-1", &kernel_id),
                &signature
            ),
            Ok(())
        );
        assert!(verify(
            &public_key,
            &aeds_hello_message("nonce-2", &kernel_id),
            &signature
        )
        .is_err());
    }

    #[test]
    fn a_management_request_is_signed_over_its_parts_with_a_fresh_nonce() {
        let identity = identity();
        let sign_once = || {
            identity
                .management_headers("PUT", "/v1/x", "dev.example", "owner-1", b"{}")
                .unwrap()
        };
        let (first, second) = (sign_once(), sign_once());
        assert_ne!(first[2].1, second[2].1, "every request has its own nonce");
        let timestamp_ms: u64 = first[1].1.parse().unwrap();
        let message = ManagementRequest {
            method: "PUT",
            path: "/v1/x",
            generator_id: "dev.example",
            owner_id: "owner-1",
            timestamp_ms,
            nonce: &first[2].1,
            body: b"{}",
        }
        .message();
        assert_eq!(verify(&first[0].1, &message, &first[3].1), Ok(()));
        assert_eq!(first[4], (OWNER_ID_HEADER, "owner-1".to_string()));
    }
}
