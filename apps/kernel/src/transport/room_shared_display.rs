//! MP-08/MP-10/MP-11: home-bound Room placement of the shared display protocol.
use serde::{Deserialize, Serialize};
// MP-11: home derives this from authenticated Room ownership, never the viewer offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoomDisplayAudience {
    Owner,
    Viewer,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayCommand {
    Attach {},
    // An unwatched display: renews the worker binding like Attach and pauses the shared desktop source. The next
    // Capture starts it again.
    Park {},
    Capture {
        after_sequence: u64,
        reset: bool,
        congested: bool,
    },
    Suspend {},
    Resume {},
    // MP-08/MP-10/MP-11: 535/143 registration invokes525 and attaches locally.
    ResumeAttach {},
    Close {},
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SealedRoomDisplayFrame {
    pub sequence: u64,
    pub encrypted_event: chariox_relay::protocol::EncryptedRelayPayload,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayResult {
    Opened {
        binding: crate::slice::RoomDisplayBinding,
        peer_public_key: String,
    },
    Frame {
        frame: Option<SealedRoomDisplayFrame>,
    },
    Updated {
        binding: crate::slice::RoomDisplayBinding,
    },
    Settled {},
}

// MP-08/MP-11: kernel-resolved references only, never viewer rectangles/XIDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RoomDisplayTarget {
    Browser {
        tab_id: String,
        generation: u64,
    },
    Desktop {
        surface_id: String,
        generation: String,
    },
    Region {
        binding_id: String,
    },
}

#[cfg(test)]
mod tests {
    use crate::transport::relay_peer::RelayPeerRequest;
    use sha2::{Digest, Sha256};

    #[test]
    fn mp11_room_display_audience_requires_new_protocol() {
        // MP-11: assignment is coordinator-owned; never ship the new audience under the old peer contract.
        const {
            assert!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION >= 558);
        };
        const {
            assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 166);
        };
    }

    #[test]
    fn mp11_room_display_audience_contract_is_versioned_and_cannot_default_to_owner() {
        assert_eq!(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            crate::test_support::expected_local_protocol_version()
        );
        assert_eq!(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let request = serde_json::json!({
            "kind":"open_room_shared_display_with_audience", "session_id":"room-1", "slice_id":"slice-1",
            "lease_id":"room-display-00000000000000000000000000000001", "viewer_public_key":"viewer-key",
            "audience":"viewer", "offer":{"codecs":["avc1.420033"],"bitrate":8000000,"device_scale_factor":1},
            "viewport":{"css_width":1280,"css_height":800,"device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800,"revision":1,"last_actor_id":null},
            "target":{"kind":"desktop","surface_id":"slice:slice-1","generation":"native-one"}
        });
        // The frozen worker accepted unknown fields on its old open variant.
        #[derive(serde::Deserialize)]
        #[serde(tag = "kind")]
        enum LegacyOpen {
            #[serde(rename = "open_room_shared_display")]
            Open {},
        }
        let mut legacy = request.clone();
        legacy["kind"] = "open_room_shared_display".into();
        assert!(serde_json::from_value::<LegacyOpen>(legacy.clone()).is_ok());
        assert!(serde_json::from_value::<RelayPeerRequest>(legacy).is_err());
        assert!(
            serde_json::from_value::<LegacyOpen>(request.clone()).is_err(),
            "MP-11 old workers must not open an unmasked feed by ignoring audience"
        );
        for audience in ["owner", "viewer"] {
            let mut value = request.clone();
            value["audience"] = audience.into();
            let decoded: RelayPeerRequest = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        }
        let mut missing = request.clone();
        missing.as_object_mut().unwrap().remove("audience");
        assert!(serde_json::from_value::<RelayPeerRequest>(missing).is_err());
        let mut unknown = request.clone();
        unknown["audience"] = "model".into();
        assert!(serde_json::from_value::<RelayPeerRequest>(unknown).is_err());
        let mut offer = request["offer"].clone();
        offer["audience"] = "owner".into();
        assert!(
            serde_json::from_value::<crate::slice::RoomDisplayOffer>(offer).is_err(),
            "MP-11 clients cannot choose the home-owned audience"
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&request).unwrap())
            ),
            "7456be3802d0f40e2441b51555cf8f09483fa32663e9b391a43e6228b936259f"
        );
    }
}
