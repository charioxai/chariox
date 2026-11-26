use super::*;

use std::net::{Ipv4Addr, SocketAddrV4};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
}

fn bearer(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {token}")).expect("bearer header should be valid")
}

fn local_token_auth() -> (KernelLocalAuth, Arc<LocalTokenAuth>) {
    let auth = Arc::new(LocalTokenAuth::new(generate_kernel_local_auth_token()));
    (KernelLocalAuth::LocalToken(Arc::clone(&auth)), auth)
}

#[test]
fn generated_tokens_are_prefixed_32_byte_base64url_and_fresh() {
    let first = generate_kernel_local_auth_token();
    let second = generate_kernel_local_auth_token();
    assert_ne!(first, second);
    let encoded = first
        .strip_prefix(KERNEL_LOCAL_AUTH_TOKEN_PREFIX)
        .expect("token should carry the recognizable prefix");
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("token body should be base64url");
    assert_eq!(decoded.len(), KERNEL_LOCAL_AUTH_TOKEN_RANDOM_BYTES);
}

#[test]
fn local_token_is_required_and_missing_or_wrong_headers_are_refused() {
    let (local_auth, auth) = local_token_auth();
    assert_eq!(
        local_auth.admit(Some(&bearer(auth.token()))),
        Some(KernelLocalCredential::LocalToken)
    );
    assert_eq!(local_auth.admit(None), None);
    for wrong in [
        bearer("chx_kat_wrong"),
        bearer(&format!("{}x", auth.token())),
        HeaderValue::from_str(auth.token()).expect("raw token header"),
        HeaderValue::from_static("Basic Zm9vOmJhcg=="),
    ] {
        assert_eq!(local_auth.admit(Some(&wrong)), None);
    }
    assert!(local_auth.required());
}

#[test]
fn host_token_stays_required_and_unconfigured_stays_unchecked() {
    let host = KernelLocalAuth::host_token_or_unconfigured(Some(Arc::from("host-sentinel")));
    assert!(host.required());
    assert_eq!(
        host.admit(Some(&bearer("host-sentinel"))),
        Some(KernelLocalCredential::HostToken)
    );
    assert_eq!(host.admit(None), None);
    assert_eq!(host.admit(Some(&bearer("wrong-host-sentinel"))), None);

    let unconfigured = KernelLocalAuth::host_token_or_unconfigured(None);
    assert!(!unconfigured.required());
    assert_eq!(
        unconfigured.admit(None),
        Some(KernelLocalCredential::Unchecked)
    );
}

#[test]
fn each_credential_admits_its_connection_class() {
    let (laptop, auth) = local_token_auth();
    let host = KernelLocalAuth::host_token_or_unconfigured(Some(Arc::from("host-sentinel")));
    let unconfigured = KernelLocalAuth::host_token_or_unconfigured(None);
    for (admitted, class) in [
        (
            laptop.admit(Some(&bearer(auth.token()))),
            KernelConnectionClass::Terminal,
        ),
        (
            host.admit(Some(&bearer("host-sentinel"))),
            KernelConnectionClass::Host,
        ),
        (
            unconfigured.admit(None),
            KernelConnectionClass::Unauthenticated,
        ),
    ] {
        assert_eq!(
            admitted.map(KernelLocalCredential::connection_class),
            Some(class)
        );
    }
}

#[test]
fn warnings_are_rate_limited_per_class_and_never_carry_a_token() {
    let (_, auth) = local_token_auth();
    let wrong_token = generate_kernel_local_auth_token();
    let start = Instant::now();
    let peer = Some(loopback(50123));

    assert!(auth
        .observe(KernelLocalCredential::LocalToken, peer, start)
        .is_none());

    let missing = auth
        .observe(KernelLocalCredential::Missing, peer, start)
        .expect("first missing token should warn");
    assert_eq!(missing.fields["credential"], "missing");
    assert_eq!(missing.fields["enforcement"], "required");
    assert_eq!(missing.fields["transport_source"], "local_cli");
    assert_eq!(missing.fields["peer_addr"], "127.0.0.1:50123");
    assert_eq!(missing.fields["peer_loopback"], true);
    assert_eq!(missing.fields["authenticated_since_start"], 1);

    let wrong = auth
        .observe(KernelLocalCredential::Wrong, peer, start)
        .expect("wrong token is a separate class and should warn");
    assert_eq!(wrong.fields["credential"], "wrong");
    assert_ne!(wrong.message, missing.message);

    assert!(auth
        .observe(
            KernelLocalCredential::Missing,
            peer,
            start + Duration::from_secs(1)
        )
        .is_none());
    assert!(auth
        .observe(
            KernelLocalCredential::Missing,
            peer,
            start + Duration::from_secs(2)
        )
        .is_none());
    let later = auth
        .observe(
            KernelLocalCredential::Missing,
            peer,
            start + LOCAL_AUTH_WARNING_INTERVAL,
        )
        .expect("missing token should warn again after the interval");
    assert_eq!(later.fields["suppressed_since_last_warning"], 2);
    assert_eq!(later.fields["total_since_start"], 4);
    assert_eq!(auth.counts(), (1, 4, 1));

    for warning in [&missing, &wrong, &later] {
        let logged = format!("{} {}", warning.message, warning.fields);
        assert!(!logged.contains(auth.token()));
        assert!(!logged.contains(&wrong_token));
        assert!(!logged.contains(KERNEL_LOCAL_AUTH_TOKEN_PREFIX));
    }
}

#[cfg(unix)]
#[test]
fn laptop_kernel_writes_a_fresh_owner_only_token_file_at_each_start() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let home = TempHome::new("token-file");

    let (first_auth, first_file) =
        KernelLocalAuth::for_local_kernel(None, Ok(loopback(43_901))).unwrap();
    let first_file = first_file.expect("token file should be written");
    let KernelLocalAuth::LocalToken(first) = &first_auth else {
        panic!("a kernel without a host token should use its local token");
    };
    let path = home
        .path()
        .join("state")
        .join("kernel-local-auth")
        .join("43901.token");
    assert_eq!(first_file.path(), path);
    assert_eq!(
        std::fs::read_to_string(&path).expect("token file should be readable"),
        format!("{}\n", first.token())
    );
    let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    let directory_mode = std::fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);
    assert_eq!(directory_mode, 0o700);

    let (second_auth, second_file) =
        KernelLocalAuth::for_local_kernel(None, Ok(loopback(43_901))).unwrap();
    let second_file = second_file.expect("restart should replace the token file");
    let KernelLocalAuth::LocalToken(second) = &second_auth else {
        panic!("a kernel without a host token should use its local token");
    };
    assert_ne!(first.token(), second.token());
    drop(first_file);
    assert_eq!(
        std::fs::read_to_string(&path).expect("the newer kernel's file should remain"),
        format!("{}\n", second.token())
    );
    drop(second_file);
    assert!(!path.exists(), "the token file is removed on shutdown");
}

#[cfg(unix)]
#[test]
fn a_loose_token_directory_is_tightened_to_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let home = TempHome::new("loose-directory");
    let directory = home.path().join("state").join("kernel-local-auth");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (_auth, file) = KernelLocalAuth::for_local_kernel(None, Ok(loopback(43_902))).unwrap();
    assert!(file.is_some());
    assert_eq!(
        std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_token_directory_is_refused() {
    let _environment = crate::env_lock::lock();
    let home = TempHome::new("symlink-directory");
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(home.path().join("state")).unwrap();
    std::os::unix::fs::symlink(
        &elsewhere,
        home.path().join("state").join("kernel-local-auth"),
    )
    .unwrap();

    let result = KernelLocalAuth::for_local_kernel(None, Ok(loopback(43_903)));
    let Err(error) = result else {
        panic!("unwritable auth must stop startup")
    };
    assert!(error
        .to_string()
        .contains("cannot write required token file"));
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn a_host_token_kernel_writes_no_local_token_file() {
    let _environment = crate::env_lock::lock();
    let home = TempHome::new("host-token");

    let (auth, file) =
        KernelLocalAuth::for_local_kernel(Some(Arc::from("host-sentinel")), Ok(loopback(43_904)))
            .unwrap();
    assert!(matches!(auth, KernelLocalAuth::HostToken(_)));
    assert!(file.is_none());
    assert!(!home.path().join("state").exists());
}

#[cfg(unix)]
#[test]
fn provider_environments_carry_neither_the_token_nor_its_path() {
    let _environment = crate::env_lock::lock();
    let _home = TempHome::new("provider-env");

    let (auth, file) = KernelLocalAuth::for_local_kernel(None, Ok(loopback(43_905))).unwrap();
    let file = file.expect("token file should be written");
    let KernelLocalAuth::LocalToken(auth) = &auth else {
        panic!("a kernel without a host token should use its local token");
    };

    let removed = crate::app::default_provider_env_remove(&DaemonConfig::for_tests());
    for name in [
        super::super::KERNEL_LOCAL_AUTH_TOKEN_ENV,
        super::super::KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV,
        "CHARIOX_HOME",
    ] {
        assert!(
            removed.iter().any(|removed| removed == name),
            "provider launches must scrub {name}"
        );
    }

    // A provider inherits the kernel's environment minus the scrubbed names.
    let mut child = std::process::Command::new("/usr/bin/env");
    for name in &removed {
        child.env_remove(name);
    }
    let output = child.output().expect("environment probe should run");
    assert!(output.status.success());
    let environment = String::from_utf8_lossy(&output.stdout);
    assert!(!environment.contains(auth.token()));
    assert!(!environment.contains(KERNEL_LOCAL_AUTH_TOKEN_PREFIX));
    assert!(!environment.contains(&*file.path().to_string_lossy()));
}

#[cfg(unix)]
struct TempHome {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl TempHome {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-local-auth-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&path).expect("temporary CHARIOX_HOME should be created");
        let previous = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &path);
        Self { path, previous }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for TempHome {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn missing_listener_address_stops_local_token_startup() {
    let result = KernelLocalAuth::for_local_kernel(
        None,
        Err(std::io::Error::other("test listener failure")),
    );
    let Err(error) = result else {
        panic!("missing address must stop startup")
    };
    assert!(error
        .to_string()
        .contains("kernel websocket address unavailable"));
}
