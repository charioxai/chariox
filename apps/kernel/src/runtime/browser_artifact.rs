//! MP-08/MP-10/MP-11: bounded common Browser artifact wire and byte provenance.
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const MAX_BROWSER_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserArtifactKind {
    Image,
    Network,
    Download,
    Identity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserArtifactRequest {
    pub target_id: String,
    pub document_id: String,
    pub browser_generation: u64,
    pub viewport: crate::session::CanonicalViewport,
    pub kind: BrowserArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserArtifactBytes {
    pub display_name: String,
    pub mime_type: String,
    pub data_base64: String,
    pub sha256: String,
    pub size_bytes: u64,
}
impl std::fmt::Debug for BrowserArtifactBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserArtifactBytes")
            .field("size_bytes", &self.size_bytes)
            .finish_non_exhaustive()
    }
}
impl BrowserArtifactBytes {
    pub fn decode(&self) -> Result<Vec<u8>, String> {
        if self.size_bytes > MAX_BROWSER_ARTIFACT_BYTES as u64
            || self.data_base64.len() > MAX_BROWSER_ARTIFACT_BYTES.div_ceil(3) * 4
            || !safe_filename(&self.display_name)
            || self.mime_type.len() > 128
        {
            return Err("Browser artifact exceeds bounded metadata/bytes".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.data_base64)
            .map_err(|_| "invalid Browser artifact base64")?;
        if bytes.len() as u64 != self.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != self.sha256
        {
            return Err("Browser artifact byte provenance mismatch".into());
        }
        Ok(bytes)
    }
}
pub(crate) fn safe_filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && ![".", ".."].contains(&value)
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserArtifactCapture {
    #[serde(flatten)]
    pub bytes: BrowserArtifactBytes,
    pub kind: BrowserArtifactKind,
    pub guid: Option<String>,
    pub target_id: String,
    pub document_id: String,
    pub browser_generation: u64,
    pub browser_id: String,
    pub viewport: crate::session::CanonicalViewport,
    pub redaction: String,
    pub geometry: serde_json::Value,
}
impl std::fmt::Debug for BrowserArtifactCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserArtifactCapture(<redacted>)")
    }
}
impl BrowserArtifactCapture {
    pub fn validate(&self, request: &BrowserArtifactRequest) -> Result<Vec<u8>, String> {
        if self.target_id != request.target_id
            || self.document_id != request.document_id
            || self.browser_generation != request.browser_generation
            || self.viewport != request.viewport
            || self.kind != request.kind
            || self.guid != request.guid
            || !self.browser_id.starts_with("browser-")
            || self.browser_id.len() != 72
            || !self.browser_id[8..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Browser artifact identity changed".into());
        }
        let geometry = self
            .geometry
            .as_object()
            .ok_or("Browser artifact geometry is absent")?;
        let fields = ["pageX", "pageY", "clientWidth", "clientHeight", "scale"];
        if geometry.len() != fields.len()
            || fields.iter().any(|key| {
                geometry
                    .get(*key)
                    .and_then(serde_json::Value::as_f64)
                    .is_none_or(|v| !v.is_finite() || v.abs() > 1_000_000_000.0)
            })
            || self.geometry["scale"].as_f64() != Some(1.0)
        {
            return Err("Browser artifact geometry is invalid".into());
        }
        let bytes = self.bytes.decode()?;
        if self.kind == BrowserArtifactKind::Image
            && (self.bytes.mime_type != "image/png"
                || bytes.len() < 24
                || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                || u32::from_be_bytes(bytes[16..20].try_into().unwrap())
                    != self.viewport.css_width * self.viewport.device_scale_factor
                || u32::from_be_bytes(bytes[20..24].try_into().unwrap())
                    != self.viewport.css_height * self.viewport.device_scale_factor
                || !matches!(
                    self.redaction.as_str(),
                    "none" | "fill_targets" | "full_viewport"
                ))
        {
            return Err("invalid canonical Browser image".into());
        }
        if matches!(
            self.kind,
            BrowserArtifactKind::Network | BrowserArtifactKind::Identity
        ) && (self.bytes.mime_type != "application/json"
            || serde_json::from_slice::<serde_json::Value>(&bytes).is_err())
        {
            return Err("Browser metadata artifact must contain bounded JSON".into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp11_browser_image_redaction_validates_and_publishes() {
        use crate::artifacts::{OperationalArtifactStore, StoreArtifactRequest};
        use std::collections::BTreeMap;
        let root = std::env::temp_dir().join(format!(
            "protxform-artifact-{:032x}",
            rand::random::<u128>()
        ));
        let store =
            OperationalArtifactStore::open(root.join("store"), root.join("index.sqlite")).unwrap();
        let viewport = crate::session::CanonicalViewport::new(8, 6, 1, 8, 6).unwrap();
        let request = BrowserArtifactRequest {
            target_id: "target".into(),
            document_id: "document".into(),
            browser_generation: 1,
            viewport: viewport.clone(),
            kind: BrowserArtifactKind::Image,
            guid: None,
        };
        let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAGCAIAAABxZ0isAAAADElEQVR4nGNgGIwAAACWAAGzNRKNAAAAAElFTkSuQmCC").unwrap();
        let mut capture = BrowserArtifactCapture {
            bytes: BrowserArtifactBytes {
                display_name: "capture.png".into(),
                mime_type: "image/png".into(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(&png),
                sha256: format!("{:x}", Sha256::digest(&png)),
                size_bytes: png.len() as u64,
            },
            kind: request.kind,
            guid: None,
            target_id: request.target_id.clone(),
            document_id: request.document_id.clone(),
            browser_generation: 1,
            browser_id: format!("browser-{}", "a".repeat(64)),
            viewport,
            geometry: serde_json::json!({"pageX":0,"pageY":0,"clientWidth":8,"clientHeight":6,"scale":1}),
            redaction: "fill_targets".into(),
        };
        for redaction in ["fill_targets", "none", "full_viewport"] {
            capture.redaction = redaction.into();
            let bytes = capture
                .validate(&request)
                .expect("MP-11 valid plain/password image must publish");
            let path = root.join("capture.png");
            std::fs::write(&path, &bytes).unwrap();
            let record = store
                .store_existing_file(StoreArtifactRequest {
                    source_path: path,
                    display_name: capture.bytes.display_name.clone(),
                    media_type: Some(capture.bytes.mime_type.clone()),
                    source_kind: "room_browser_artifact".into(),
                    enqueue_archive: false,
                    session_id: Some("room".into()),
                    attachment_id: None,
                    workspace_id: None,
                    worktree_path: None,
                    metadata: BTreeMap::from_iter([(
                        "redaction".into(),
                        capture.redaction.clone().into(),
                    )]),
                })
                .unwrap();
            let loaded = store.load_artifact(&record.artifact_id).unwrap().unwrap();
            assert_eq!(loaded.metadata["redaction"], redaction);
            assert_eq!(std::fs::read(loaded.operational_path).unwrap(), bytes);
        }
        capture.redaction = "unrecognized".into();
        assert!(capture.validate(&request).is_err());
        // MP-08/MP-11: opt-in exact real-CDP captures from the DPR1/2 browser drill.
        if let Some(fixtures) = std::env::var_os("CHARIOX_BROWSER_ARTIFACT_FIXTURES") {
            for dpr in [1, 2] {
                for (name, redaction) in [
                    ("password", "none"),
                    ("plain", "fill_targets"),
                    ("retired", "none"),
                ] {
                    let path = std::path::Path::new(&fixtures)
                        .join(format!("artifact-dpr{dpr}-{name}.json"));
                    let capture: BrowserArtifactCapture =
                        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                    assert_eq!(capture.redaction, redaction);
                    let request = BrowserArtifactRequest {
                        target_id: capture.target_id.clone(),
                        document_id: capture.document_id.clone(),
                        browser_generation: capture.browser_generation,
                        viewport: capture.viewport.clone(),
                        kind: capture.kind,
                        guid: capture.guid.clone(),
                    };
                    let bytes = capture
                        .validate(&request)
                        .expect("MP-11 real Chromium capture must validate");
                    let path = root.join(format!("dpr{dpr}-{name}.png"));
                    std::fs::write(&path, &bytes).unwrap();
                    let record = store
                        .store_existing_file(StoreArtifactRequest {
                            source_path: path,
                            display_name: capture.bytes.display_name.clone(),
                            media_type: Some(capture.bytes.mime_type.clone()),
                            source_kind: "room_browser_artifact".into(),
                            enqueue_archive: false,
                            session_id: Some("room".into()),
                            attachment_id: None,
                            workspace_id: None,
                            worktree_path: None,
                            metadata: BTreeMap::from_iter([(
                                "redaction".into(),
                                capture.redaction.into(),
                            )]),
                        })
                        .unwrap();
                    let loaded = store.load_artifact(&record.artifact_id).unwrap().unwrap();
                    assert_eq!(loaded.metadata["redaction"], redaction);
                    assert_eq!(std::fs::read(loaded.operational_path).unwrap(), bytes);
                }
            }
        }
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn mp08_mp10_mp11_browser_artifact_bytes_are_hash_size_and_name_bound() {
        let mut value = BrowserArtifactBytes {
            display_name: "report.txt".into(),
            mime_type: "text/plain".into(),
            data_base64: "aGk=".into(),
            size_bytes: 2,
            sha256: format!("{:x}", Sha256::digest(b"hi")),
        };
        assert_eq!(value.decode().unwrap(), b"hi");
        value.size_bytes = 3;
        assert!(value.decode().is_err());
        value.size_bytes = 2;
        value.sha256 = "0".repeat(64);
        assert!(value.decode().is_err());
        value.display_name = "../private".into();
        assert!(value.decode().is_err());
        assert!(!safe_filename("."));
        assert!(!safe_filename("a\nb"));
    }
}
