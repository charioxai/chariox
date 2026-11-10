//! `displayName` and `icon`: optional signed presentation of an App.
use super::*;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(data);
    out.extend([0; 4]);
    out
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let mut out = PNG_SIGNATURE.to_vec();
    out.extend(chunk(b"IHDR", &header));
    out.extend(chunk(b"IDAT", &[0; 8]));
    out.extend(chunk(b"IEND", &[]));
    out
}

const SVG: &str = r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><rect width="16" height="16" fill="url(#g)"/><use href="#g"/></svg>"##;

fn with_icon(path: &str, bytes: Vec<u8>) -> Fixture {
    let mut fixture = base();
    fixture.manifest.icon = Some(path.to_owned());
    fixture.files.insert(path.to_owned(), bytes);
    fixture
}

fn pack_error(fixture: &Fixture) -> Option<ErrorCode> {
    pack(
        &fixture.manifest,
        &fixture.files,
        &key(),
        &[],
        &Limits::default(),
    )
    .err()
    .map(|error| error.code)
}

#[test]
fn a_package_without_display_fields_stays_valid() {
    let verified_bytes = packed(&base(), &[]);
    let verified = verify(&verified_bytes, &policy()).unwrap();
    assert!(verified.manifest().display_name.is_none());
    assert!(verified.manifest().icon.is_none());
    assert!(verified.icon_data_url().is_none());
}

#[test]
fn display_name_is_trimmed_bounded_text_and_signed() {
    let mut fixture = base();
    let plain = verify(&packed(&fixture, &[]), &policy())
        .unwrap()
        .digests()
        .content
        .clone();
    fixture.manifest.display_name = Some("Todo list".into());
    let bytes = packed(&fixture, &[]);
    let verified = verify(&bytes, &policy()).unwrap();
    assert_eq!(
        verified.manifest().display_name.as_deref(),
        Some("Todo list")
    );
    assert_ne!(verified.digests().content, plain);
    for name in ["Ünïcode ✓", &"x".repeat(64)] {
        fixture.manifest.display_name = Some(name.to_string());
        assert!(
            fixture.manifest.validate(&Limits::default()).is_ok(),
            "{name}"
        );
    }
    for name in [
        "",
        " Todo",
        "Todo ",
        "   ",
        "bell\u{7}",
        "two\nlines",
        "tab\there",
        "nul\0",
        &"x".repeat(65),
    ] {
        fixture.manifest.display_name = Some(name.to_string());
        assert!(
            fixture.manifest.validate(&Limits::default()).is_err(),
            "{name:?}"
        );
    }
}

#[test]
fn a_png_icon_is_served_as_a_data_url_from_the_signed_package() {
    let fixture = with_icon("ui/icon.png", png(64, 64));
    let bytes = packed(&fixture, &[]);
    let verified = verify(&bytes, &policy()).unwrap();
    let url = verified.icon_data_url().unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        url.strip_prefix("data:image/png;base64,").unwrap(),
    )
    .unwrap();
    assert_eq!(decoded, png(64, 64));
}

#[test]
fn an_svg_icon_without_active_content_is_accepted() {
    let fixture = with_icon("icon.svg", SVG.as_bytes().to_vec());
    let verified_bytes = packed(&fixture, &[]);
    let verified = verify(&verified_bytes, &policy()).unwrap();
    assert!(verified
        .icon_data_url()
        .unwrap()
        .starts_with("data:image/svg+xml;base64,"));
}

#[test]
fn an_icon_must_be_a_listed_small_png_or_svg() {
    let mut missing = base();
    missing.manifest.icon = Some("ui/absent.png".into());
    assert!(pack_error(&missing).is_some());

    for path in ["ui/icon.gif", "ui/icon", "ui/icon.png.html", "../icon.png"] {
        assert!(pack_error(&with_icon(path, png(8, 8))).is_some(), "{path}");
    }

    let mut oversized = png(8, 8);
    oversized.splice(33..33, vec![0; 32 * 1024]);
    assert!(pack_error(&with_icon("ui/icon.png", oversized)).is_some());

    for broken in [
        b"not a png".to_vec(),
        png(0, 8),
        png(8, 0),
        png(513, 8),
        png(8, 513),
        png(8, 8)[..40].to_vec(),
        PNG_SIGNATURE.to_vec(),
    ] {
        assert!(pack_error(&with_icon("ui/icon.png", broken)).is_some());
    }
    assert!(pack_error(&with_icon("ui/icon.png", png(512, 512))).is_none());
    assert!(pack_error(&with_icon("ui/icon.svg", png(8, 8))).is_some());
}

#[test]
fn an_svg_icon_with_scripts_or_external_references_is_refused() {
    let refused = [
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><SCRIPT>alert(1)</SCRIPT></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><rect ONCLICK = 'x()'/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div/></foreignObject></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><iframe src="x"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/a.png"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><use xlink:href="https://example.com/a.svg#x"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="data:image/png;base64,AAAA"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><a href="javascript:alert(1)"><rect/></a></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><a href="java&#115;cript:alert(1)"><rect/></a></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="fill:url(https://example.com/x)"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>@import url(https://example.com/x.css);</style></svg>"#,
        r#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg"/>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="href" values="javascript:alert(1)"/></svg>"#,
        r#"<html><body/></html>"#,
        "",
    ];
    for svg in refused {
        let fixture = with_icon("ui/icon.svg", svg.as_bytes().to_vec());
        assert!(pack_error(&fixture).is_some(), "{svg}");
    }
    let invalid_utf8 = with_icon("ui/icon.svg", vec![b'<', b's', b'v', b'g', 0xff, b'>']);
    assert!(pack_error(&invalid_utf8).is_some());
}

/// Signs like `pack` but skips its content checks, as a hostile signer could.
fn sign_without_pack_checks(fixture: &Fixture) -> Vec<u8> {
    use crate::{json, Inventory, InventoryFile, PublisherSignature};
    use ed25519_dalek::Signer;
    let limits = Limits::default();
    let inventory = Inventory {
        schema: INTEGRITY_SCHEMA_V2.to_owned(),
        files: fixture
            .files
            .iter()
            .map(|(path, bytes)| InventoryFile {
                path: path.clone(),
                size: bytes.len() as u64,
                digest: crate::digest(bytes),
            })
            .collect(),
    };
    let manifest_value = serde_json::to_value(&fixture.manifest).unwrap();
    let signed = signing_bytes_for(SIGNATURE_SCHEMA_V2, &manifest_value, &inventory).unwrap();
    let signature = PublisherSignature {
        key_id: fixture.manifest.publisher.key_id.clone(),
        algorithm: "ed25519".to_owned(),
        digest: crate::digest(&signed),
        value: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            key().sign(&signed).to_bytes(),
        ),
    };
    let manifest_bytes = json::canonical(&manifest_value).unwrap();
    let inventory_bytes = json::canonical(&inventory).unwrap();
    let signature_bytes = json::canonical(&signature).unwrap();
    let mut entries: Vec<(&str, &[u8])> = vec![
        (archive::CONTROL_PATHS[0], &manifest_bytes),
        (archive::CONTROL_PATHS[1], &inventory_bytes),
        (archive::CONTROL_PATHS[2], &signature_bytes),
    ];
    entries.extend(
        fixture
            .files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    archive::encode(&entries, &limits).unwrap()
}

#[test]
fn verify_alone_refuses_a_signed_package_whose_icon_is_unsafe() {
    let mut fixture = with_icon("ui/icon.svg", SVG.as_bytes().to_vec());
    assert!(verify(&sign_without_pack_checks(&fixture), &policy()).is_ok());
    fixture.files.insert(
        "ui/icon.svg".into(),
        br#"<svg xmlns="http://www.w3.org/2000/svg" onload="x()"/>"#.to_vec(),
    );
    assert!(pack_error(&fixture).is_some());
    let bytes = sign_without_pack_checks(&fixture);
    assert!(verify(&bytes, &policy()).is_err());
    fixture.manifest.display_name = Some(" untrimmed".into());
    fixture
        .files
        .insert("ui/icon.svg".into(), SVG.as_bytes().to_vec());
    assert!(verify(&sign_without_pack_checks(&fixture), &policy()).is_err());
}
