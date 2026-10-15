//! MP-08 / MP-10 / MP-11: shared private-file exclusion for instruction/Extension payloads.
pub(crate) fn sensitive_package_path(path: &str) -> bool {
    path.split('/').any(|component| {
        let component = component.to_ascii_lowercase();
        component.starts_with(".env")
            || matches!(
                component.as_str(),
                ".npmrc"
                    | ".pypirc"
                    | ".netrc"
                    | ".git-credentials"
                    | ".ssh"
                    | "id_rsa"
                    | "id_ed25519"
                    | "credentials.json"
                    | "credentials.yaml"
                    | "credentials.yml"
                    | "secrets.json"
                    | "secrets.yaml"
                    | "secrets.yml"
            )
            || component.ends_with(".pem")
            || component.ends_with(".p12")
            || component.ends_with(".pfx")
            || component.ends_with(".key")
    })
}

pub(crate) fn contains_private_key(bytes: &[u8]) -> bool {
    bytes
        .windows(b"PRIVATE KEY-----".len())
        .any(|window| window == b"PRIVATE KEY-----")
}
