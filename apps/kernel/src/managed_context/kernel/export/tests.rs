use super::*;

fn test_vault_snapshot() -> crate::secret::TransferredVaultSnapshot {
    let source_private = crate::transport::relay_crypto::generate_private_key_base64();
    let source_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&source_private)
            .expect("source public key should derive");
    let target_private = crate::transport::relay_crypto::generate_private_key_base64();
    let target_public =
        crate::transport::relay_crypto::public_key_from_private_key_base64(&target_private)
            .expect("target public key should derive");
    let sealed_unlock_key = crate::transport::relay_crypto::encrypt_payload_for_peer(
        &source_private,
        &target_public,
        &[7_u8; 32],
    )
    .expect("Vault key should seal");
    let vault = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "kdf": {
            "algorithm": "argon2id",
            "salt": base64::engine::general_purpose::STANDARD.encode([3_u8; 16]),
            "memory_kib": 65_536,
            "iterations": 3,
            "parallelism": 1
        },
        "cipher": "aes-256-gcm",
        "nonce": base64::engine::general_purpose::STANDARD.encode([4_u8; 12]),
        "ciphertext": base64::engine::general_purpose::STANDARD.encode([5_u8; 16])
    }))
    .expect("Vault file should serialize");
    crate::secret::TransferredVaultSnapshot {
        schema_version: 1,
        context_id: "context-one".to_string(),
        source_kernel_id: "source-kernel".to_string(),
        source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
            &source_public,
        ),
        target_kernel_id: "target-kernel".to_string(),
        target_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
            &target_public,
        ),
        vault_sha256: sha256_hex(&vault),
        vault_size_bytes: vault.len() as u64,
        vault_file_base64: base64::engine::general_purpose::STANDARD.encode(vault),
        sealed_unlock_key,
    }
}

fn test_export_request() -> KernelContextExportRequest {
    let vault = test_vault_snapshot();
    KernelContextExportRequest {
        context_id: vault.context_id.clone(),
        source_kernel_id: vault.source_kernel_id.clone(),
        source_key_thumbprint: vault.source_key_thumbprint.clone(),
        target_kernel_id: vault.target_kernel_id.clone(),
        target_key_thumbprint: vault.target_key_thumbprint.clone(),
        vault,
    }
}

fn test_root(name: String) -> PathBuf {
    fs::canonicalize(std::env::temp_dir())
        .expect("temporary root should canonicalize")
        .join(name)
}

#[test]
fn kernel_context_exports_one_unified_extension_set() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-export-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);

    let mcp_root = isolation.join("user/mcps");
    fs::create_dir_all(&mcp_root).expect("MCP root should create");
    let mut docs_mcp = CharioxMcpServerConfig::streamable_http("docs", "https://example.test/mcp");
    if let CharioxMcpTransportConfig::StreamableHttp { http_headers, .. } = &mut docs_mcp.transport
    {
        http_headers.insert("Content-Type".to_string(), "application/json".to_string());
        http_headers.insert("API-Version".to_string(), "2026-08-20".to_string());
    }
    fs::write(
        mcp_root.join("docs.json"),
        serde_json::to_vec(&docs_mcp).expect("MCP should serialize"),
    )
    .expect("MCP should write");

    let skill_root = isolation.join("user/skills/review");
    fs::create_dir_all(&skill_root).expect("skill root should create");
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\nReview carefully.\n",
    )
    .expect("skill should write");
    fs::write(skill_root.join("review.sh"), "#!/bin/sh\n").expect("skill helper should write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            skill_root.join("review.sh"),
            fs::Permissions::from_mode(0o700),
        )
        .expect("skill helper should become executable");
    }

    let script_root = isolation.join("user/scripts/check");
    fs::create_dir_all(&script_root).expect("script root should create");
    fs::write(script_root.join("script.py"), "print('ok')\n").expect("script should write");
    fs::write(
            script_root.join("metadata.json"),
            r#"{"name":"check","runtime":"python","entrypoint":"script.py","description":"Check","input_schema":{},"definition_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .expect("script metadata should write");
    let env_root = isolation.join("user/envs");
    fs::create_dir_all(&env_root).expect("env root should create");
    fs::write(
        env_root.join("python.json"),
        r#"{"name":"python","runtime":{"type":"python","python":"/usr/bin/python3"}}"#,
    )
    .expect("environment should write");
    let portable_python = env_root.join(".portable/python");
    fs::create_dir_all(&portable_python).expect("portable Python root should create");
    fs::write(
        portable_python.join("manifest.json"),
        r#"{"schema_version":1,"runtime":"python","version":"3.12.1"}"#,
    )
    .expect("portable Python manifest should write");
    fs::write(portable_python.join("requirements.lock"), "")
        .expect("Python requirements lock should write");
    let portable_node = env_root.join(".portable/node");
    fs::create_dir_all(&portable_node).expect("portable Node root should create");
    fs::write(
        env_root.join("node.json"),
        serde_json::to_vec(&crate::script::CharioxEnvironmentConfig {
            name: "node".to_string(),
            runtime: CharioxEnvironmentRuntime::Node {
                node: PathBuf::from("/usr/bin/node"),
                package_root: Some(portable_node.clone()),
            },
        })
        .expect("Node environment should serialize"),
    )
    .expect("Node environment should write");
    fs::write(
        portable_node.join("manifest.json"),
        r#"{"schema_version":1,"runtime":"node","version":"22.11.0"}"#,
    )
    .expect("portable Node manifest should write");
    fs::write(
        portable_node.join("package.json"),
        r#"{"name":"chariox-env","version":"1.0.0"}"#,
    )
    .expect("portable Node package should write");
    fs::write(
        portable_node.join("package-lock.json"),
        r#"{"name":"chariox-env","version":"1.0.0","lockfileVersion":3,"packages":{}}"#,
    )
    .expect("portable Node lock should write");
    let node_modules = portable_node.join("node_modules");
    fs::create_dir_all(&node_modules).expect("installed Node tree should create");
    fs::File::create(node_modules.join("large-installed-artifact"))
        .and_then(|file| file.set_len(128_u64 * 1024 * 1024 + 1))
        .expect("large installed artifact should create");
    #[cfg(unix)]
    std::os::unix::fs::symlink("../tool/bin.js", node_modules.join(".bin"))
        .expect("installed Node link should create");

    let adapter_root = chariox_home.join("connectors/adapters/http");
    fs::create_dir_all(&adapter_root).expect("adapter root should create");
    fs::write(
            adapter_root.join("adapter.yaml"),
            "kind: connector_adapter\nname: http\nadapter_protocol: chariox-connector-adapter-v2\ncommand: ./adapter.sh\n",
        )
        .expect("adapter should write");
    fs::write(adapter_root.join("adapter.sh"), "#!/bin/sh\n").expect("adapter script");
    let unused_adapter_root = chariox_home.join("connectors/adapters/unused");
    fs::create_dir_all(&unused_adapter_root).expect("unused adapter root should create");
    fs::write(
            unused_adapter_root.join("adapter.yaml"),
            "kind: connector_adapter\nname: unused\nadapter_protocol: chariox-connector-adapter-v2\ncommand: ./unused.sh\n",
        )
        .expect("unused adapter should write");
    fs::write(unused_adapter_root.join("unused.sh"), "#!/bin/sh\n")
        .expect("unused adapter script should write");
    let connector_root = chariox_home.join("connectors/definitions");
    fs::create_dir_all(&connector_root).expect("connector root should create");
    fs::write(
            connector_root.join("status.yaml"),
            "kind: connector\nname: status\ndescription: Status connector\nadapter: http\noperations:\n  - name: get\n    description: Get status\n    input_schema: {type: object}\n",
        )
        .expect("connector should write");

    let credential_root = chariox_home.join("credentials");
    fs::create_dir_all(&credential_root).expect("credential root should create");
    fs::write(
            credential_root.join("api.yaml"),
            "id: api\nsource: {type: vault, key: api}\nallowed_uses: [connector]\ninjection: {kind: header, name: Authorization, value: 'Bearer ${secret}'}\nmetadata: {created_by_kind: agent, created_by_id: agent-secret-id, session_id: session-secret-id, provider_run_id: provider-run-secret-id}\n",
        )
        .expect("credential should write");

    let snapshot =
        export_kernel_context(test_export_request()).expect("kernel context should export");
    assert_eq!(snapshot.payload.extensions.len(), 4);
    assert!(snapshot
        .payload
        .extensions
        .iter()
        .all(|extension| extension.scope == KernelExtensionScope::User));
    assert!(snapshot.payload.extensions.iter().all(|extension| {
        extension_definition_hash(&extension.definition)
            .is_ok_and(|hash| hash == extension.definition_sha256)
    }));
    assert_eq!(
        snapshot
            .payload
            .extensions
            .iter()
            .map(|extension| extension.kind.clone())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ExtensionKind::Mcp,
            ExtensionKind::Skill,
            ExtensionKind::Script,
            ExtensionKind::Connector,
        ])
    );
    assert!(snapshot
        .payload
        .dependencies
        .iter()
        .any(|dependency| matches!(dependency, KernelExtensionDependency::Credential { .. })));
    assert!(snapshot
        .payload
        .dependencies
        .iter()
        .any(|dependency| matches!(
            dependency,
            KernelExtensionDependency::UserConnectorAdapter { .. }
        )));
    assert!(snapshot
        .payload
        .dependencies
        .iter()
        .any(|dependency| matches!(
            dependency,
            KernelExtensionDependency::Environment {
                runtime: PortableEnvironmentRuntime::Python { version, .. },
                ..
            } if version == "3.12.1"
        )));
    assert!(snapshot
        .payload
        .dependencies
        .iter()
        .any(|dependency| matches!(
            dependency,
            KernelExtensionDependency::Environment {
                name,
                runtime: PortableEnvironmentRuntime::Node { version, .. },
            } if name == "node" && version == "22.11.0"
        )));
    assert_eq!(
        snapshot
            .payload
            .dependencies
            .iter()
            .filter(|dependency| matches!(
                dependency,
                KernelExtensionDependency::UserConnectorAdapter { .. }
            ))
            .count(),
        1
    );
    #[cfg(unix)]
    assert!(snapshot.payload.extensions.iter().any(|extension| matches!(
        &extension.definition,
        KernelExtensionDefinition::Skill {
            package,
            executable_paths,
        } if package.metadata.path == PathBuf::from("SKILL.md")
            && executable_paths == &["review.sh".to_string()]
    )));
    assert_eq!(snapshot.snapshot_sha256.len(), 64);
    let serialized = serde_json::to_string(&snapshot).expect("snapshot should serialize");
    assert!(!serialized.contains("session-secret-id"));
    assert!(!serialized.contains("provider-run-secret-id"));
    let debug = format!("{snapshot:?}");
    for canary in [
        "Review carefully",
        "print('ok')",
        "Bearer ${secret}",
        "agent-secret-id",
    ] {
        assert!(!debug.contains(canary), "Debug leaked `{canary}`");
    }

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_stdio_mcp_without_signed_runtime_artifact() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-reject-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let mcp_root = isolation.join("user/mcps");
    fs::create_dir_all(&mcp_root).expect("MCP root should create");
    let mut mcp = CharioxMcpServerConfig::stdio("local", "/private/tool", Vec::new());
    if let CharioxMcpTransportConfig::Stdio { cwd, .. } = &mut mcp.transport {
        *cwd = Some(PathBuf::from("/private/workspace"));
    }
    fs::write(
        mcp_root.join("local.json"),
        serde_json::to_vec(&mcp).expect("MCP should serialize"),
    )
    .expect("MCP should write");
    let error =
        export_kernel_context(test_export_request()).expect_err("host-specific MCP should reject");
    assert!(error
        .to_string()
        .contains("has no signed portable runtime artifact"));
    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_http_mcp_credential_fragments() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-http-fragment-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let mcp_root = isolation.join("user/mcps");
    fs::create_dir_all(&mcp_root).expect("MCP root should create");
    let mcp = CharioxMcpServerConfig::streamable_http(
        "private",
        "https://example.test/mcp#access_token=secret-canary",
    );
    fs::write(
        mcp_root.join("private.json"),
        serde_json::to_vec(&mcp).expect("MCP should serialize"),
    )
    .expect("MCP should write");

    let error = export_kernel_context(test_export_request())
        .expect_err("fragment-bearing MCP URL should reject");
    assert!(error.to_string().contains("use credential bindings"));
    assert!(!format!("{error:?}").contains("secret-canary"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_literal_http_mcp_credentials() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-http-secret-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let mcp_root = isolation.join("user/mcps");
    fs::create_dir_all(&mcp_root).expect("MCP root should create");
    let mut mcp = CharioxMcpServerConfig::streamable_http(
        "private",
        "https://example.test/mcp?sig=secret-canary",
    );
    if let CharioxMcpTransportConfig::StreamableHttp { http_headers, .. } = &mut mcp.transport {
        http_headers.insert("X-Auth".to_string(), "secret-canary".to_string());
    }
    fs::write(
        mcp_root.join("private.json"),
        serde_json::to_vec(&mcp).expect("MCP should serialize"),
    )
    .expect("MCP should write");

    let error = export_kernel_context(test_export_request())
        .expect_err("literal MCP credential should reject");
    assert!(error.to_string().contains("use credential bindings"));
    assert!(!format!("{error:?}").contains("secret-canary"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_prefixed_connector_auth_fields() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-connector-secret-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::env::set_var(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        root.join("capabilities"),
    );
    std::env::set_var("CHARIOX_HOME", root.join("home"));
    let connector_root = root.join("home/connectors/definitions");
    fs::create_dir_all(&connector_root).expect("connector root should create");
    for (config, expected_error) in [
        (
            "http_auth: {value: secret-canary}",
            "literal credential-like fields",
        ),
        (
            "http_auth: [secret-canary]",
            "literal credential-like fields",
        ),
        (
            "http_auth: {url: 'https://identity.example.test/#access_token=secret-canary'}",
            "literal credential-like fields",
        ),
        (
            "endpoint: 'https://api.example.test/#access_token=secret-canary'",
            "credential-bearing URL",
        ),
    ] {
        fs::write(
            connector_root.join("private.yaml"),
            format!(
                "kind: connector\nname: private\ndescription: Private connector\nadapter: http\noperations:\n  - name: get\n    description: Get private data\n    input_schema: {{type: object}}\n    config: {{{config}}}\n"
            ),
        )
        .expect("connector should write");

        let error = export_kernel_context(test_export_request())
            .expect_err("nested connector auth field should reject");
        assert!(error.to_string().contains(expected_error));
        assert!(!format!("{error:?}").contains("secret-canary"));
    }

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_non_vault_credential_definitions() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-credential-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let credential_root = chariox_home.join("credentials");
    fs::create_dir_all(&credential_root).expect("credential root should create");
    fs::write(
            credential_root.join("ambient.yaml"),
            "id: ambient\nsource: {type: env, name: API_TOKEN}\nallowed_uses: [connector]\ninjection: {kind: header, name: Authorization, value: 'Bearer {secret}'}\n",
        )
        .expect("credential should write");

    let error =
        export_kernel_context(test_export_request()).expect_err("ambient credential should reject");
    assert!(error.to_string().contains("nonportable env or file source"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_literal_vault_credential_injection() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-credential-literal-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::env::set_var(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        root.join("capabilities"),
    );
    std::env::set_var("CHARIOX_HOME", root.join("home"));
    let credential_root = root.join("home/credentials");
    fs::create_dir_all(&credential_root).expect("credential root should create");
    fs::write(
        credential_root.join("literal.yaml"),
        "id: literal\nsource: {type: vault, key: api}\nallowed_uses: [connector]\ninjection: {kind: header, name: Authorization, value: 'Bearer hardcoded-token'}\n",
    )
    .expect("credential should write");

    let error = export_kernel_context(test_export_request())
        .expect_err("literal credential injection should reject");
    assert!(error.to_string().contains("`${secret}` template"));
    assert!(!format!("{error:?}").contains("hardcoded-token"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_registry_paths_outside_private_capture() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-path-escape-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let canary = root.join("host-canary.py");
    fs::create_dir_all(&root).expect("test root should create");
    fs::write(&canary, "print('host-canary-secret')\n").expect("host canary should write");

    let script_root = isolation.join("user/scripts/escape");
    fs::create_dir_all(&script_root).expect("script root should create");
    fs::write(
        script_root.join("metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": "escape",
            "runtime": "python",
            "entrypoint": canary,
            "description": "Escape",
            "input_schema": {},
            "definition_hash": "a".repeat(64)
        }))
        .expect("script metadata should serialize"),
    )
    .expect("script metadata should write");
    let error = export_kernel_context(test_export_request())
        .expect_err("absolute script entrypoint should reject");
    assert!(error.to_string().contains("escaped its captured registry"));
    assert!(!format!("{error:?}").contains("host-canary-secret"));

    fs::remove_dir_all(isolation.join("user/scripts")).expect("script root should remove");
    let env_root = isolation.join("user/envs");
    fs::create_dir_all(&env_root).expect("environment root should create");
    fs::write(
        env_root.join("escape.json"),
        r#"{"name":"../host-canary","runtime":{"type":"python","python":"/usr/bin/python3"}}"#,
    )
    .expect("environment should write");
    let error = export_kernel_context(test_export_request())
        .expect_err("traversing environment name should reject");
    assert!(error.to_string().contains("environment name"));
    assert!(!format!("{error:?}").contains("host-canary-secret"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_structurally_invalid_vault_snapshot() {
    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-invalid-vault-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::env::set_var(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        root.join("capabilities"),
    );
    std::env::set_var("CHARIOX_HOME", root.join("home"));
    let mut request = test_export_request();
    request.vault.vault_sha256 = "0".repeat(64);
    let error = export_kernel_context(request).expect_err("invalid Vault digest should reject");
    assert!(error.to_string().contains("declared digest"));
    let mut request = test_export_request();
    request.vault.sealed_unlock_key.nonce = "not-base64".to_string();
    let error = export_kernel_context(request).expect_err("invalid sealed payload should reject");
    assert!(error.to_string().contains("relay nonce"));
    let mut request = test_export_request();
    let malformed = b"not-json";
    request.vault.vault_file_base64 = base64::engine::general_purpose::STANDARD.encode(malformed);
    request.vault.vault_size_bytes = malformed.len() as u64;
    request.vault.vault_sha256 = sha256_hex(malformed);
    let error = export_kernel_context(request)
        .expect_err("self-consistent malformed Vault file should reject");
    assert!(error
        .to_string()
        .contains("parse transferred Chariox Vault"));
    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_requires_reproducible_environment_inputs() {
    let root = test_root(format!(
        "chariox-kernel-context-environment-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let python_root = root.join(".portable/python");
    fs::create_dir_all(&python_root).expect("portable Python root should create");
    fs::write(
        python_root.join("manifest.json"),
        r#"{"schema_version":1,"runtime":"python","version":"3.12.1"}"#,
    )
    .expect("Python manifest should write");
    fs::write(python_root.join("requirements.lock"), "").expect("Python lock should write");
    let error = export_portable_environment(
        &root,
        Some(&root),
        &crate::script::CharioxEnvironmentConfig {
            name: "python".to_string(),
            runtime: CharioxEnvironmentRuntime::Python {
                python: PathBuf::from("/tmp/private-venv/bin/python"),
            },
        },
    )
    .expect_err("custom Python runtime should reject");
    assert!(error.to_string().contains("reproducible Python runtime"));

    let node_root = root.join(".portable/node");
    fs::create_dir_all(&node_root).expect("portable Node root should create");
    fs::write(
        node_root.join("manifest.json"),
        r#"{"schema_version":1,"runtime":"node","version":"22.11.0"}"#,
    )
    .expect("Node manifest should write");
    fs::write(
        node_root.join("package.json"),
        r#"{"name":"chariox-env","version":"1.0.0"}"#,
    )
    .expect("package.json should write");
    fs::write(
        node_root.join("package-lock.json"),
        r#"{"name":"chariox-env","version":"1.0.0","lockfileVersion":3,"packages":{}}"#,
    )
    .expect("package lock should write");
    fs::write(
        node_root.join(".npmrc"),
        "//registry/:_authToken=secret-canary\n",
    )
    .expect("unselected npm config should write");
    let runtime = export_portable_environment(
        &root,
        Some(&root),
        &crate::script::CharioxEnvironmentConfig {
            name: "node".to_string(),
            runtime: CharioxEnvironmentRuntime::Node {
                node: PathBuf::from("/usr/bin/node"),
                package_root: Some(node_root.clone()),
            },
        },
    )
    .expect("pinned Node environment should export");
    assert!(matches!(
        runtime,
        PortableEnvironmentRuntime::Node { version, files }
            if version == "22.11.0"
                && files.len() == 3
                && files.iter().all(|file| file.path != ".npmrc")
    ));
    for auth_value in [
        serde_json::json!({"value": "secret-canary"}),
        serde_json::json!(["secret-canary"]),
    ] {
        fs::write(
            node_root.join("package-lock.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": "chariox-env",
                "version": "1.0.0",
                "lockfileVersion": 3,
                "npmAuth": auth_value,
                "packages": {
                    "node_modules/private": {
                        "integrity": "sha512-ZmFrZQ==",
                        "resolved": "https://registry.example.test/pkg"
                    }
                }
            }))
            .expect("credential-bearing package lock should serialize"),
        )
        .expect("credential-bearing package lock should write");
        let error = export_portable_environment(
            &root,
            Some(&root),
            &crate::script::CharioxEnvironmentConfig {
                name: "node".to_string(),
                runtime: CharioxEnvironmentRuntime::Node {
                    node: PathBuf::from("/usr/bin/node"),
                    package_root: Some(node_root.clone()),
                },
            },
        )
        .expect_err("nested credential-bearing package metadata should reject");
        assert!(!format!("{error:?}").contains("secret-canary"));
    }
    fs::write(
        node_root.join("package-lock.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": "chariox-env",
            "version": "1.0.0",
            "lockfileVersion": 3,
            "packages": {
                "node_modules/private": {
                    "integrity": "sha512-ZmFrZQ==",
                    "resolved": "https://registry.example.test/pkg#access_token=secret-canary"
                }
            }
        }))
        .expect("fragment-bearing package lock should serialize"),
    )
    .expect("fragment-bearing package lock should write");
    let error = export_portable_environment(
        &root,
        Some(&root),
        &crate::script::CharioxEnvironmentConfig {
            name: "node".to_string(),
            runtime: CharioxEnvironmentRuntime::Node {
                node: PathBuf::from("/usr/bin/node"),
                package_root: Some(node_root),
            },
        },
    )
    .expect_err("fragment-bearing package URL should reject");
    assert!(!format!("{error:?}").contains("secret-canary"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kernel_context_rejects_embedded_host_paths_and_package_aliases() {
    assert!(argument_has_host_path("--config=/Users/me/private.json"));
    assert!(argument_has_host_path(
        "--config=C:\\Users\\me\\private.json"
    ));
    assert!(argument_has_host_path("../private.json"));
    assert!(!argument_has_host_path("--config=./config.json"));
    assert!(validate_portable_package_paths(["a/b", "a\\b"]).is_err());
    assert!(validate_portable_package_paths(["Docs/readme", "docs/README"]).is_err());
    assert!(validate_portable_package_paths(["CON.txt"]).is_err());
    assert!(validate_portable_package_paths([".env.production"]).is_err());
    assert!(validate_safe_package_file("tool.pem", b"not-even-a-real-key").is_err());
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "api_token": "secret-canary"
    }))
    .is_err());
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "endpoint": "https://example.test/data?X-Amz-Security-Token=secret-canary"
    }))
    .is_err());
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "x_api_key": "secret-canary"
    }))
    .is_err());
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "AWSAccessKeyId": "secret-canary",
        "endpoint": "https://example.test/data?Signature=secret-canary"
    }))
    .is_err());
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "secret_key": "secret-canary",
        "api_secret_key": "secret-canary",
        "passphrase": "secret-canary",
        "http_auth": "secret-canary"
    }))
    .is_err());
    for nested_auth in [
        serde_json::json!({"http_auth": {"value": "secret-canary"}}),
        serde_json::json!({"http_auth": ["secret-canary"]}),
        serde_json::json!({
            "http_auth": {
                "url": "https://identity.example.test/#access_token=secret-canary"
            }
        }),
    ] {
        assert!(reject_literal_secrets_in_json(&nested_auth).is_err());
    }
    for name in [
        "x-api-key",
        "x_auth_token",
        "api-secret",
        "X-Amz-Credential",
        "X-Amz-Signature",
        "X-Amz-Security-Token",
        "AWSAccessKeyId",
        "aws_secret_access_key",
        "aws_session_token",
        "github_api_key",
        "my_api_key",
        "Signature",
        "auth",
        "authentication",
        "secret_key",
        "api_secret_key",
        "passphrase",
        "sig",
        "X-Auth",
        "http_auth",
        "npmAuth",
    ] {
        assert!(sensitive_credential_field(name), "missed `{name}`");
    }
    assert!(reject_literal_secrets_in_json(&serde_json::json!({
        "continuation_token": "cursor-value",
        "token_budget": 1_000,
        "secret_enabled": false,
        "http_auth": false,
        "authentication": "oauth2",
        "nested_auth": {
            "mode": "oauth2",
            "issuer": "https://identity.example.test/",
            "credential_ref": "api_binding",
            "enabled": true
        },
        "supported_auth": ["oauth2", "https://identity.example.test/"]
    }))
    .is_ok());
}

#[test]
fn bundled_adapter_artifact_hash_covers_implementation_bytes() {
    let root = test_root(format!(
        "chariox-kernel-context-bundled-adapter-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    fs::create_dir_all(&root).expect("adapter root should create");
    let manifest = root.join("adapter.yaml");
    fs::write(
            &manifest,
            "kind: connector_adapter\nname: bundled\nadapter_protocol: chariox-connector-adapter-v2\ncommand: ./adapter.sh\n",
        )
        .expect("adapter manifest should write");
    fs::write(root.join("adapter.sh"), "#!/bin/sh\necho one\n").expect("adapter should write");
    let adapter = CharioxConnectorAdapterDefinition {
        kind: "connector_adapter".to_string(),
        name: "bundled".to_string(),
        version: Some("1.0.0".to_string()),
        adapter_protocol: crate::connector::CONNECTOR_ADAPTER_PROTOCOL_VERSION.to_string(),
        command: PathBuf::from("./adapter.sh"),
        args: Vec::new(),
        description: None,
        source: Some(ConnectorAdapterSource::Bundled),
        manifest_path: Some(manifest),
    };
    let first = bundled_adapter_artifact_hash(&adapter).expect("artifact should hash");
    fs::write(root.join("adapter.sh"), "#!/bin/sh\necho two\n").expect("adapter should change");
    let second = bundled_adapter_artifact_hash(&adapter).expect("artifact should rehash");
    assert_ne!(first, second);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn kernel_context_rejects_symlinks_in_extension_roots() {
    use std::os::unix::fs::symlink;

    let _guard = crate::env_lock::lock();
    let root = test_root(format!(
        "chariox-kernel-context-symlink-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let isolation = root.join("capabilities");
    let chariox_home = root.join("home");
    std::env::set_var("CHARIOX_CAPABILITY_ISOLATION_ROOT", &isolation);
    std::env::set_var("CHARIOX_HOME", &chariox_home);
    let skill_root = isolation.join("user/skills/review");
    fs::create_dir_all(&skill_root).expect("skill root should create");
    fs::write(
        skill_root.join("SKILL.md"),
        "---\nname: review\ndescription: Review code\n---\n",
    )
    .expect("skill should write");
    let outside = root.join("outside.txt");
    fs::write(&outside, "do not copy\n").expect("outside file should write");
    symlink(&outside, skill_root.join("outside.txt")).expect("symlink should create");

    let error = export_kernel_context(test_export_request()).expect_err("symlink should reject");
    assert!(error.to_string().contains("symlink"));

    std::env::remove_var("CHARIOX_CAPABILITY_ISOLATION_ROOT");
    std::env::remove_var("CHARIOX_HOME");
    let _ = fs::remove_dir_all(root);
}
