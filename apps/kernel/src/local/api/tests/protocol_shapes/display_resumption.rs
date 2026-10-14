//! MP-08/MP-10/MP-11: allocated 525/133 shared producer resumption contract.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserRequest};
use sha2::{Digest, Sha256};

#[test]
fn mp08_mp10_mp11_display_resumption_525_133_snapshot_hash() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 525);
    assert_eq!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION, 133);
    let requests = [
        KernelBrowserCommand::DisplaySuspend {
            subscription_id: "delivery-old".into(),
            generation: 7,
        },
        KernelBrowserCommand::DisplayResume {
            subscription_id: "delivery-old".into(),
            generation: 7,
        },
    ]
    .into_iter()
    .map(|command| LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command }))
    .map(|request| serde_json::to_value(request).unwrap())
    .collect::<Vec<_>>();
    let snapshot = serde_json::json!({
        "local_protocol": LOCAL_DAEMON_PROTOCOL_VERSION,
        "peer_protocol": crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        "requests": requests,
    });
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&snapshot).unwrap())),
        "8a3eb16c3a2cbbd170cbd7d608a990f5994b0cff57f9047440d93bed0a0f7674"
    );
    for op in ["display_suspend", "display_resume"] {
        assert!(serde_json::from_value::<KernelBrowserCommand>(serde_json::json!({
            "op":op,"subscription_id":"delivery-old","generation":7,"observed_by":"forged"
        })).is_err(), "caller identity must come from transport admission");
    }
}
