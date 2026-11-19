//! MP-08/MP-10: protocol 1012 names each Tab's icon by `favicon_hash`, so a Browser state read carries an
//! icon's PNG only to a viewer that does not hold it yet (`known_favicons` on `StateWait`).
use super::kernel_browser_runtime::host_error;
use crate::error::DaemonError;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A viewer holds at most this many icons; it lists only the icons of the Tabs it shows.
const MAX_KNOWN: usize = 128;

/// Checks a viewer's icon list once, at the request boundary.
pub(super) fn known_favicons(known: &[String]) -> Result<&[String], DaemonError> {
    let valid = known.len() <= MAX_KNOWN
        && known.iter().all(|hash| {
            hash.len() == 16
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    if !valid {
        return Err(host_error(format!(
            "MP-08: known_favicons lists at most {MAX_KNOWN} favicon_hash values (16 lowercase hex)"
        )));
    }
    Ok(known)
}

/// Gives every Tab in `state` its `favicon_hash` (null without an icon) and keeps its PNG only when
/// `known` does not name it. A plain read (`known` is `None`) never carries a PNG: its callers show none.
pub(super) fn name_favicons(state: &mut Value, known: Option<&[String]>) {
    let Some(tabs) = state.get_mut("tabs").and_then(Value::as_array_mut) else {
        return;
    };
    for tab in tabs.iter_mut().filter_map(Value::as_object_mut) {
        let png = tab.remove("favicon_png_base64");
        let hash = png.as_ref().and_then(Value::as_str).map(favicon_hash);
        if let (Some(hash), Some(known), Some(png)) = (&hash, known, png) {
            if !known.contains(hash) {
                tab.insert("favicon_png_base64".into(), png);
            }
        }
        tab.insert("favicon_hash".into(), hash.into());
    }
}

fn favicon_hash(png: &str) -> String {
    Sha256::digest(png.as_bytes())[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state() -> Value {
        json!({"generation":1,"tabs":[
            {"tab_id":"a","favicon_png_base64":"iVBORw0KGgoA"},
            {"tab_id":"b","favicon_png_base64":null},
        ]})
    }

    #[test]
    fn a_viewer_gets_only_the_icons_it_does_not_hold() {
        let hash = favicon_hash("iVBORw0KGgoA");
        let mut fresh = state();
        name_favicons(&mut fresh, Some(&[]));
        assert_eq!(
            fresh["tabs"],
            json!([
                {"tab_id":"a","favicon_png_base64":"iVBORw0KGgoA","favicon_hash":hash},
                {"tab_id":"b","favicon_hash":null},
            ])
        );
        let mut held = state();
        name_favicons(&mut held, Some(std::slice::from_ref(&hash)));
        assert_eq!(held["tabs"][0], json!({"tab_id":"a","favicon_hash":hash}));
        let mut plain = state();
        name_favicons(&mut plain, None);
        assert_eq!(plain["tabs"][0], json!({"tab_id":"a","favicon_hash":hash}));
    }

    #[test]
    fn a_viewer_lists_bounded_lowercase_hashes() {
        assert!(known_favicons(&["0123456789abcdef".into()]).is_ok());
        for bad in ["0123456789ABCDEF", "0123456789abcde", "0123456789abcdefa"] {
            assert!(known_favicons(&[bad.into()]).is_err(), "{bad}");
        }
        assert!(known_favicons(&vec!["0123456789abcdef".to_string(); MAX_KNOWN + 1]).is_err());
    }
}
