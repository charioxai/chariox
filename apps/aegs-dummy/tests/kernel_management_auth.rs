//! The real dummy AEGS on loopback: a kernel manages its connections by signing each request
//! with its identity key and naming its secret owner id. No token, no Cloud.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_event_protocol::kernel_identity::{
    kernel_id_for_public_key, sign, ManagementRequest, KERNEL_KEY_HEADER, KERNEL_NONCE_HEADER,
    KERNEL_SIGNATURE_HEADER, KERNEL_TIMESTAMP_HEADER, OWNER_ID_HEADER,
};
use serde_json::{json, Value};

const GENERATOR: &str = "dev.chariox.dummy";
const INTEREST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

struct Aegs {
    child: Child,
    port: u16,
    dir: std::path::PathBuf,
}

impl Aegs {
    fn start() -> Self {
        Self::start_with(&[])
    }

    fn start_with(env: &[(&str, String)]) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = std::env::temp_dir().join(format!("aegs-kernel-auth-{port}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_chariox-aegs-dummy"));
        command
            .env("CHARIOX_AEGS_ADDR", format!("127.0.0.1:{port}"))
            .env("CHARIOX_AEGS_DATABASE_PATH", dir.join("aegs.db"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (name, value) in env {
            command.env(name, value);
        }
        let aegs = Self {
            child: command.spawn().expect("the dummy AEGS starts"),
            port,
            dir,
        };
        for _ in 0..100 {
            if ureq::get(&format!("http://127.0.0.1:{port}/healthz"))
                .call()
                .is_ok()
            {
                return aegs;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the dummy AEGS did not become healthy");
    }
}

impl Drop for Aegs {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Kernel {
    signing: p256::ecdsa::SigningKey,
    private: String,
    public: String,
}

/// What a request carries; the defaults are a correct, fresh signed request.
struct Call<'a> {
    method: &'a str,
    path: &'a str,
    owner_header: Option<&'a str>,
    body: Value,
    timestamp_ms: Option<u64>,
    nonce: Option<&'a str>,
    signed_body: Option<&'a str>,
    signer: Option<&'a Kernel>,
}

impl Kernel {
    fn new() -> Self {
        let signing = p256::ecdsa::SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let public = STANDARD.encode(signing.verifying_key().to_encoded_point(false).as_bytes());
        Self {
            private: STANDARD.encode(signing.to_bytes()),
            signing,
            public,
        }
    }

    fn id(&self) -> String {
        kernel_id_for_public_key(&self.public)
    }

    fn call(&self, aegs: &Aegs, call: Call) -> (u16, Value) {
        let body = call.body.to_string();
        let signer = call.signer.unwrap_or(self);
        let timestamp_ms = call.timestamp_ms.unwrap_or_else(now_ms);
        let nonce = call
            .nonce
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:032x}", rand_u128()));
        let signed = call.signed_body.unwrap_or(&body);
        let signature = sign(
            &signer.private,
            &ManagementRequest {
                method: call.method,
                path: call.path,
                generator_id: GENERATOR,
                owner_id: call.owner_header.unwrap_or(""),
                timestamp_ms,
                nonce: &nonce,
                body: signed.as_bytes(),
            }
            .message(),
        )
        .unwrap();
        let mut request = ureq::request(
            call.method,
            &format!("http://127.0.0.1:{}{}", aegs.port, call.path),
        )
        .set(KERNEL_KEY_HEADER, &self.public)
        .set(KERNEL_TIMESTAMP_HEADER, &timestamp_ms.to_string())
        .set(KERNEL_NONCE_HEADER, &nonce)
        .set(KERNEL_SIGNATURE_HEADER, &signature)
        .set("content-type", "application/json");
        if let Some(owner) = call.owner_header {
            request = request.set(OWNER_ID_HEADER, owner);
        }
        let result = if call.method == "GET" {
            request.call()
        } else {
            request.send_string(&body)
        };
        let (status, response) = match result {
            Ok(response) => (response.status(), response),
            Err(ureq::Error::Status(status, response)) => (status, response),
            Err(error) => panic!("management transport failed: {error}"),
        };
        (status, response.into_json().unwrap_or(Value::Null))
    }

    fn post(&self, aegs: &Aegs, path: &str, owner: &str, body: Value) -> (u16, Value) {
        self.call(aegs, Call::new("POST", path, Some(owner), body))
    }

    fn authorize(&self, aegs: &Aegs, owner: &str) -> String {
        let (status, flow) = self.post(
            aegs,
            "/v1/authorizations",
            owner,
            json!({"generator_id": GENERATOR, "owner_id": owner}),
        );
        assert_eq!(status, 200, "{flow}");
        flow["connection_id"].as_str().unwrap().to_string()
    }

    fn reconcile(&self, aegs: &Aegs, owner: &str, connection: &str, binding: &str) -> (u16, Value) {
        self.call(
            aegs,
            Call::new(
                "PUT",
                "/v1/subscriptions/reconcile",
                Some(owner),
                json!({"owner_id": owner, "generator_id": GENERATOR, "subscriptions": [{
                    "binding_id": binding, "generator_id": GENERATOR, "connection_id": connection,
                    "connection_scope": "default", "event_interest_key": INTEREST,
                    "event_type": "dummy.triggered", "event_type_version": 1,
                    "filter": null, "revision": 1, "active": true,
                }]}),
            ),
        )
    }
}

impl<'a> Call<'a> {
    fn new(method: &'a str, path: &'a str, owner: Option<&'a str>, body: Value) -> Self {
        Self {
            method,
            path,
            owner_header: owner,
            body,
            timestamp_ms: None,
            nonce: None,
            signed_body: None,
            signer: None,
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn rand_u128() -> u128 {
    use p256::elliptic_curve::rand_core::RngCore;
    let mut bytes = [0_u8; 16];
    p256::elliptic_curve::rand_core::OsRng.fill_bytes(&mut bytes);
    u128::from_be_bytes(bytes)
}

fn error_code(response: &Value) -> &str {
    response["error"]["code"].as_str().unwrap_or("")
}

const OWNER: &str = "owner-secret-0000000000000000001";

#[test]
fn a_signed_request_with_an_owner_id_manages_the_connection() {
    let aegs = Aegs::start();
    let kernel = Kernel::new();
    let connection = kernel.authorize(&aegs, OWNER);
    let (status, body) = kernel.reconcile(&aegs, OWNER, &connection, "bind-1");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accepted_binding_ids"], json!(["bind-1"]));
}

#[test]
fn the_static_operator_token_no_longer_opens_management() {
    let aegs = Aegs::start_with(&[(
        "CHARIOX_AEGS_MANAGEMENT_TOKEN",
        "operator-token".to_string(),
    )]);
    let result = ureq::post(&format!("http://127.0.0.1:{}/v1/authorizations", aegs.port))
        .set("authorization", "Bearer operator-token")
        .set(OWNER_ID_HEADER, OWNER)
        .send_json(json!({"generator_id": GENERATOR, "owner_id": OWNER}));
    let status = match result {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(
        status, 401,
        "a bearer token must not authenticate management"
    );
}

#[test]
fn a_second_kernel_with_the_owner_id_joins_and_manages_the_same_connection() {
    let aegs = Aegs::start();
    let (first, second) = (Kernel::new(), Kernel::new());
    let connection = first.authorize(&aegs, OWNER);
    let (status, body) = second.reconcile(&aegs, OWNER, &connection, "bind-second");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accepted_binding_ids"], json!(["bind-second"]));
    let (status, set) = first.call(
        &aegs,
        Call::new("GET", "/v1/owners/kernels", Some(OWNER), Value::Null),
    );
    assert_eq!(status, 200, "{set}");
    let kernels: Vec<&str> = set["kernels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|kernel| kernel["state"] == "member")
        .map(|kernel| kernel["kernel_id"].as_str().unwrap())
        .collect();
    assert_eq!(kernels.len(), 2, "{set}");
    assert!(kernels.contains(&first.id().as_str()) && kernels.contains(&second.id().as_str()));
}

#[test]
fn a_kernel_with_a_valid_key_and_another_owner_id_gets_nothing() {
    let aegs = Aegs::start();
    let (owner_kernel, stranger) = (Kernel::new(), Kernel::new());
    let connection = owner_kernel.authorize(&aegs, OWNER);
    let (status, body) = stranger.reconcile(
        &aegs,
        "owner-guess-000000000000000002",
        &connection,
        "bind-x",
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accepted_binding_ids"], json!([]), "{body}");
    let (_, listing) = owner_kernel.call(
        &aegs,
        Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null),
    );
    assert_eq!(listing["subscriptions"], json!([]), "{listing}");
    // The stranger cannot even read the owner's connection by naming it.
    let (_, connections) = stranger.post(
        &aegs,
        "/v1/connections/query",
        "owner-guess-000000000000000002",
        json!({"generator_id": GENERATOR, "owner_id": "owner-guess-000000000000000002"}),
    );
    assert_eq!(connections["connections"], json!([]), "{connections}");
}

#[test]
fn a_request_without_an_owner_id_is_refused() {
    let aegs = Aegs::start();
    let kernel = Kernel::new();
    let (status, body) = kernel.call(
        &aegs,
        Call::new("GET", "/v1/subscriptions", None, Value::Null),
    );
    assert_eq!(
        (status, error_code(&body)),
        (403, "owner_required"),
        "{body}"
    );
}

#[test]
fn a_body_owner_that_differs_from_the_signed_owner_is_refused() {
    let aegs = Aegs::start();
    let kernel = Kernel::new();
    let (status, body) = kernel.post(
        &aegs,
        "/v1/authorizations",
        OWNER,
        json!({"generator_id": GENERATOR, "owner_id": "owner-other-0000000000000000003"}),
    );
    assert_eq!(
        (status, error_code(&body)),
        (403, "management_owner_mismatch"),
        "{body}"
    );
}

#[test]
fn a_replayed_request_is_refused() {
    let aegs = Aegs::start();
    let kernel = Kernel::new();
    let call = || {
        let mut call = Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null);
        call.timestamp_ms = Some(now_ms());
        call.nonce = Some("fixed-nonce-0001");
        call
    };
    // The same timestamp must be reused, so fix it once.
    let timestamp = now_ms();
    let mut first = call();
    first.timestamp_ms = Some(timestamp);
    let mut second = call();
    second.timestamp_ms = Some(timestamp);
    assert_eq!(kernel.call(&aegs, first).0, 200);
    let (status, body) = kernel.call(&aegs, second);
    assert_eq!(
        (status, error_code(&body)),
        (401, "kernel_signature_replayed"),
        "{body}"
    );
}

#[test]
fn a_request_signed_for_another_body_or_a_stale_time_or_by_another_key_is_refused() {
    let aegs = Aegs::start();
    let (kernel, other) = (Kernel::new(), Kernel::new());
    let mut tampered = Call::new(
        "POST",
        "/v1/authorizations",
        Some(OWNER),
        json!({"generator_id": GENERATOR, "owner_id": OWNER}),
    );
    tampered.signed_body = Some("{}");
    let (status, body) = kernel.call(&aegs, tampered);
    assert_eq!(
        (status, error_code(&body)),
        (401, "kernel_signature_invalid"),
        "{body}"
    );

    let mut stale = Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null);
    stale.timestamp_ms = Some(now_ms() - 10 * 60_000);
    let (status, body) = kernel.call(&aegs, stale);
    assert_eq!(
        (status, error_code(&body)),
        (401, "kernel_signature_stale"),
        "{body}"
    );

    // The request names `kernel`'s key but `other` holds the signature: a key mismatch.
    let mut forged = Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null);
    forged.signer = Some(&other);
    let (status, body) = kernel.call(&aegs, forged);
    assert_eq!(
        (status, error_code(&body)),
        (401, "kernel_signature_invalid"),
        "{body}"
    );
}

#[test]
fn a_revoked_kernel_key_is_refused_and_its_subscriptions_stop() {
    let aegs = Aegs::start();
    let (first, second) = (Kernel::new(), Kernel::new());
    let connection = first.authorize(&aegs, OWNER);
    assert_eq!(
        second.reconcile(&aegs, OWNER, &connection, "bind-second").0,
        200
    );

    let (status, body) = first.post(
        &aegs,
        "/v1/owners/kernels/revoke",
        OWNER,
        json!({"owner_id": OWNER, "kernel_id": second.id()}),
    );
    assert_eq!(status, 200, "{body}");

    let (status, body) = second.reconcile(&aegs, OWNER, &connection, "bind-second");
    assert_eq!(
        (status, error_code(&body)),
        (403, "kernel_revoked"),
        "{body}"
    );
    // A revoked key cannot rejoin by presenting the owner id again.
    let (status, body) = second.call(
        &aegs,
        Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null),
    );
    assert_eq!(
        (status, error_code(&body)),
        (403, "kernel_revoked"),
        "{body}"
    );
    // Its subscription no longer counts for the owner.
    let (_, listing) = first.call(
        &aegs,
        Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null),
    );
    assert!(
        listing["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|subscription| subscription["active"] == false),
        "{listing}"
    );
}

#[test]
fn the_allowlist_admits_only_listed_kernel_ids() {
    let (listed, unlisted) = (Kernel::new(), Kernel::new());
    let dir = std::env::temp_dir().join(format!("aegs-allowlist-{}", rand_u128()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("allowlist.json");
    std::fs::write(&file, json!({listed.id(): "listed kernel"}).to_string()).unwrap();
    let aegs = Aegs::start_with(&[
        ("CHARIOX_AEGS_ADMISSION", "allowlist".to_string()),
        (
            "CHARIOX_AEGS_ALLOWLIST_FILE",
            file.to_string_lossy().to_string(),
        ),
    ]);
    listed.authorize(&aegs, OWNER);
    let (status, body) = unlisted.post(
        &aegs,
        "/v1/authorizations",
        "owner-other-0000000000000000004",
        json!({"generator_id": GENERATOR, "owner_id": "owner-other-0000000000000000004"}),
    );
    assert_eq!(
        (status, error_code(&body)),
        (403, "kernel_not_admitted"),
        "{body}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_admission_caps_the_owners_one_kernel_may_create() {
    let aegs = Aegs::start_with(&[
        ("CHARIOX_AEGS_ADMISSION", "open".to_string()),
        ("CHARIOX_AEGS_OPEN_MAX_OWNERS_PER_KERNEL", "1".to_string()),
    ]);
    let kernel = Kernel::new();
    kernel.authorize(&aegs, OWNER);
    let (status, body) = kernel.post(
        &aegs,
        "/v1/authorizations",
        "owner-second-00000000000000005",
        json!({"generator_id": GENERATOR, "owner_id": "owner-second-00000000000000005"}),
    );
    assert_eq!(
        (status, error_code(&body)),
        (403, "owner_limit_reached"),
        "{body}"
    );
    // The first owner keeps working.
    assert_eq!(
        kernel
            .reconcile(&aegs, OWNER, &format!("local-dummy-{OWNER}"), "bind-1")
            .0,
        200
    );
}

#[test]
fn open_admission_caps_the_request_rate_of_one_kernel() {
    let aegs = Aegs::start_with(&[
        ("CHARIOX_AEGS_ADMISSION", "open".to_string()),
        ("CHARIOX_AEGS_OPEN_MAX_REQUESTS_PER_MINUTE", "3".to_string()),
    ]);
    let kernel = Kernel::new();
    let statuses: Vec<u16> = (0..6)
        .map(|_| {
            kernel
                .call(
                    &aegs,
                    Call::new("GET", "/v1/subscriptions", Some(OWNER), Value::Null),
                )
                .0
        })
        .collect();
    assert_eq!(&statuses[..3], [200, 200, 200], "{statuses:?}");
    assert!(
        statuses[3..].iter().all(|status| *status == 429),
        "{statuses:?}"
    );
}
