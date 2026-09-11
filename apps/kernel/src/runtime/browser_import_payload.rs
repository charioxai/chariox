//! Secret delivery buffer. Not a terminal request or authorization token.
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::Deserializer;
use zeroize::Zeroizing;

const MAX_BYTES: usize = 512 * 1024;
const MAX_COOKIES: usize = 512;

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
}

impl std::fmt::Debug for BrowserImportPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted browser import payload]")
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
        let raw = r#"[{"name":"session","value":"synthetic-secret"}]"#;
        let payload = BrowserImportPayload::new(Zeroizing::new(raw.into())).unwrap();
        assert_eq!(payload.as_str(), raw);
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
