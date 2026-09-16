//! Secret delivery buffer. Not a terminal request or authorization token.
use base64::Engine;
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::{Zeroize, Zeroizing};

use crate::local::BrowserImportSelection;

const MAX_BYTES: usize = 512 * 1024;
const MAX_COOKIES: usize = 512;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BrowserImportPayload(Zeroizing<String>);

impl BrowserImportPayload {
    /// Validate the outer resource bounds without materializing additional
    /// cookie-value strings. The destination must also validate cookie semantics
    /// against the kernel-owned consent scope before any mutation.
    pub(crate) fn new(value: Zeroizing<String>) -> Result<Self, &'static str> {
        if value.len() > MAX_BYTES {
            return Err("browser_import_payload_invalid");
        }
        let mut decoder = serde_json::Deserializer::from_str(&value);
        decoder
            .deserialize_seq(BoundedCookies)
            .and_then(|_| decoder.end())
            .map_err(|_| "browser_import_payload_invalid")?;
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn from_base64(mut encoded: String) -> Result<Self, &'static str> {
        if encoded.len() > MAX_BYTES.saturating_mul(4).saturating_add(2) / 3 + 4 {
            encoded.zeroize();
            return Err("browser_import_payload_invalid");
        }
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes());
        encoded.zeroize();
        let mut decoded = Zeroizing::new(decoded.map_err(|_| "browser_import_payload_invalid")?);
        std::str::from_utf8(&decoded).map_err(|_| "browser_import_payload_invalid")?;
        let value = unsafe { String::from_utf8_unchecked(std::mem::take(&mut *decoded)) };
        Self::new(Zeroizing::new(value))
    }
}

impl std::fmt::Debug for BrowserImportPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted browser import payload]")
    }
}

impl Serialize for BrowserImportPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BrowserImportPayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Zeroizing::new(String::deserialize(deserializer)?);
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

struct EncodedPayload(BrowserImportPayload);

impl<'de> Deserialize<'de> for EncodedPayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        BrowserImportPayload::from_base64(encoded)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserImportDeliveryRequest {
    pub(crate) request_id: String,
    pub(crate) selection: BrowserImportSelection,
    payload_base64: EncodedPayload,
}

impl BrowserImportDeliveryRequest {
    pub(crate) fn into_parts(self) -> (String, BrowserImportSelection, BrowserImportPayload) {
        (self.request_id, self.selection, self.payload_base64.0)
    }
}

impl std::fmt::Debug for BrowserImportDeliveryRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserImportDeliveryRequest")
            .field("request_id", &"[browser import request]")
            .field("selection", &self.selection)
            .field("payload", &"[redacted browser import payload]")
            .finish()
    }
}

struct BoundedCookies;
impl<'de> Visitor<'de> for BoundedCookies {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded cookie array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        let mut count = 0;
        while sequence.next_element::<IgnoredAny>()?.is_some() {
            count += 1;
            if count > MAX_COOKIES {
                return Err(serde::de::Error::custom("browser_import_payload_invalid"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_debug_is_redacted_and_content_is_preserved_only_for_delivery() {
        let generated_value = format!("generated-{}", crate::session::unix_epoch_ms());
        let raw = serde_json::json!([{"name":"session","value":generated_value}]).to_string();
        let payload = BrowserImportPayload::new(Zeroizing::new(raw.clone())).unwrap();
        assert_eq!(payload.as_str(), raw);
        assert!(!format!("{payload:?}").contains(&generated_value));
        assert_eq!(format!("{payload:?}"), "[redacted browser import payload]");
    }

    #[test]
    fn payload_rejects_oversize_excess_records_and_invalid_json_with_fixed_errors() {
        for raw in [
            "x".repeat(MAX_BYTES + 1),
            format!("[{}]", vec!["{}"; 513].join(",")),
            "{}".into(),
            "[secret".into(),
            "[] trailing-secret".into(),
        ] {
            assert_eq!(
                BrowserImportPayload::new(Zeroizing::new(raw)).unwrap_err(),
                "browser_import_payload_invalid"
            );
        }
        assert!(BrowserImportPayload::new(Zeroizing::new(format!(
            "[{}]",
            vec!["{}"; 512].join(",")
        )))
        .is_ok());
    }
}
