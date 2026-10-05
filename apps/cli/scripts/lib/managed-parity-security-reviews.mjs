// MP-11: independent complete-blob reviews at the exact current OSS candidate.
// Scope review, not live parity or independent scanner-tool approval. No repins.
export const SECURITY_SEMANTIC_REVIEWS = Object.freeze([
  {
    "id": "mp11narrow-52ac7421125b00479da1",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/secret/vault.rs",
      "blob": "68a5afb03e568e5ae7969e78deebda1df8811c44",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "credentials",
      "contextHash": "30e0cac53f3d60f407c45c27aaee1dfff3d70011292f5d4d8f7e38ae74a4288e"
    },
    "disposition": "removal_required",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-52ac7421125b00479da1-review",
      "reviewedAt": "2026-10-05T05:38:42.860271+00:00",
      "verdict": "FINDING",
      "severity": "P2",
      "enforces": "Vault, credential and provider-account material handling",
      "check": "Enforces encrypted Vault storage, unlock leases, passkey commitments and bound transfer. Read the complete implementation and embedded tests: AES-GCM random nonce, Argon2 parameter/size limits, no-clobber transfer and source/target AAD, key zeroization, re-key locking and expiry removal in secret reads. Confirmed F1: extend obtains the cached key without testing is_expired; caller checks status before awaiting management consent (runtime_vault_unlock_state.rs:405-428). Add atomic expiry rejection/removal under the same unlock mutex before extension. Embedded tests were read, not executed; no live secrecy or transfer claim.",
      "inspectedRanges": [
        [
          1,
          2419
        ]
      ],
      "rationale": "MP-11: FINDING. P2. Enforces encrypted Vault storage, unlock leases, passkey commitments and bound transfer. Read the complete implementation and embedded tests: AES-GCM random nonce, Argon2 parameter/size limits, no-clobber transfer and source/target AAD, key zeroization, re-key locking and expiry removal in secret reads. Confirmed F1: extend obtains the cached key without testing is_expired; caller checks status before awaiting management consent (runtime_vault_unlock_state.rs:405-428). Add atomic expiry rejection/removal under the same unlock mutex before extension. Embedded tests were read, not executed; no live secrecy or transfer claim."
    }
  },
  {
    "id": "mp11narrow-f8c56db616cce2591729",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/secret/vault.rs",
      "blob": "68a5afb03e568e5ae7969e78deebda1df8811c44",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "access",
      "contextHash": "30e0cac53f3d60f407c45c27aaee1dfff3d70011292f5d4d8f7e38ae74a4288e"
    },
    "disposition": "removal_required",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-f8c56db616cce2591729-review",
      "reviewedAt": "2026-10-05T05:38:42.860604+00:00",
      "verdict": "FINDING",
      "severity": "P2",
      "enforces": "Kernel access, sudo and passkey gates",
      "check": "Enforces encrypted Vault storage, unlock leases, passkey commitments and bound transfer. Read the complete implementation and embedded tests: AES-GCM random nonce, Argon2 parameter/size limits, no-clobber transfer and source/target AAD, key zeroization, re-key locking and expiry removal in secret reads. Confirmed F1: extend obtains the cached key without testing is_expired; caller checks status before awaiting management consent (runtime_vault_unlock_state.rs:405-428). Add atomic expiry rejection/removal under the same unlock mutex before extension. Embedded tests were read, not executed; no live secrecy or transfer claim.",
      "inspectedRanges": [
        [
          1,
          2419
        ]
      ],
      "rationale": "MP-11: FINDING. P2. Enforces encrypted Vault storage, unlock leases, passkey commitments and bound transfer. Read the complete implementation and embedded tests: AES-GCM random nonce, Argon2 parameter/size limits, no-clobber transfer and source/target AAD, key zeroization, re-key locking and expiry removal in secret reads. Confirmed F1: extend obtains the cached key without testing is_expired; caller checks status before awaiting management consent (runtime_vault_unlock_state.rs:405-428). Add atomic expiry rejection/removal under the same unlock mutex before extension. Embedded tests were read, not executed; no live secrecy or transfer claim."
    }
  },
  {
    "id": "mp11narrow-fa13cab460aaf645d750",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/provider/account_credential.rs",
      "blob": "5d778dae6d16e534c9e1ffd3b509368e8b2aaac2",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "credentials",
      "contextHash": "b680cc557f58817fa576faaecb3bc87e5504e5cc126ea3283f9f35456c543801"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-fa13cab460aaf645d750-review",
      "reviewedAt": "2026-10-05T05:38:42.867022+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Vault, credential and provider-account material handling",
      "check": "Enforces per-owner/provider/profile credential handles and provider-only Vault registration. Read the full file and embedded tests: identity includes three NUL-separated components hashed into a registry-safe handle; launch resolution returns the secret-only ProviderCredentialEnvironment, only Claude setup credentials are supported, empty material is rejected, replacement is explicit, and usable product-linked Claude login avoids a redundant Vault token. Owner validation in registry callers and real account login remain separate unreviewed dependencies.",
      "inspectedRanges": [
        [
          1,
          448
        ]
      ],
      "rationale": "MP-11: OK. Enforces per-owner/provider/profile credential handles and provider-only Vault registration. Read the full file and embedded tests: identity includes three NUL-separated components hashed into a registry-safe handle; launch resolution returns the secret-only ProviderCredentialEnvironment, only Claude setup credentials are supported, empty material is rejected, replacement is explicit, and usable product-linked Claude login avoids a redundant Vault token. Owner validation in registry callers and real account login remain separate unreviewed dependencies."
    }
  },
  {
    "id": "mp11narrow-95acec4f6995dc1bf3e9",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/provider/account_handoff.rs",
      "blob": "1f3ff3613c6ea15e93e0b03be7a59403a7c80df1",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "credentials",
      "contextHash": "97768966350c835732181124222a203cbae1649c9df3ad7444fd6f4230241387"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-95acec4f6995dc1bf3e9-review",
      "reviewedAt": "2026-10-05T05:38:42.872804+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Vault, credential and provider-account material handling",
      "check": "Enforces byte-length framing for account-switch prompt context and user request. Read the complete decoder and Unicode/tag-injection negative tests: checked str::get boundaries reject malformed UTF-8 cuts and overflow, exact delimiters bind context/request sizes and return the trailing attachment suffix separately. This helper handles prompt framing, not provider credential acquisition; broader path classification is conservative.",
      "inspectedRanges": [
        [
          1,
          61
        ]
      ],
      "rationale": "MP-11: OK. Enforces byte-length framing for account-switch prompt context and user request. Read the complete decoder and Unicode/tag-injection negative tests: checked str::get boundaries reject malformed UTF-8 cuts and overflow, exact delimiters bind context/request sizes and return the trailing attachment suffix separately. This helper handles prompt framing, not provider credential acquisition; broader path classification is conservative."
    }
  },
  {
    "id": "mp11narrow-c444bb49e835d16a8b49",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/provider/credential_environment.rs",
      "blob": "04d6a7c05c82780757e5572664474bdc3247e9e4",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "credentials",
      "contextHash": "15f1c3bee2c28659a6d8802152f1bf8b553562daf1993ae0aa4175e0870e7a33"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-c444bb49e835d16a8b49-review",
      "reviewedAt": "2026-10-05T05:38:42.879615+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Vault, credential and provider-account material handling",
      "check": "Enforces in-memory-only provider launch credentials. Read the full type and test helpers: values and clones use Zeroizing, Drop wipes and clears, Debug emits count only, and no serde implementation allows persistence/projection. Test probes are cfg(test). Correct provider process delivery and other copies remain separate obligations.",
      "inspectedRanges": [
        [
          1,
          221
        ]
      ],
      "rationale": "MP-11: OK. Enforces in-memory-only provider launch credentials. Read the full type and test helpers: values and clones use Zeroizing, Drop wipes and clears, Debug emits count only, and no serde implementation allows persistence/projection. Test probes are cfg(test). Correct provider process delivery and other copies remain separate obligations."
    }
  },
  {
    "id": "mp11narrow-992dbf4a82fcc4ac1cb4",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/relay/src/auth.rs",
      "blob": "0473218f250e736e1c9254ad7ddb9d304342c399",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "relay",
      "contextHash": "c25be468e2fa1208d1ff2a81ddd4a2fa0157e62c4533cc9b6bb0ddec715058b9"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-992dbf4a82fcc4ac1cb4-review",
      "reviewedAt": "2026-10-05T05:38:42.885774+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Relay admission, scoped tokens, peer identity and protocol gating",
      "check": "Enforces signed scoped-token action/target/expiry/revocation admission. Read full verifier, decoder and claim validation: JWT requires HS256 and JWT type, signature covers header+claims using selected issuer HMAC, unknown actions reject, expiry and future issue checks fail closed on broken clock, live revocations are account/kind scoped, and comparisons use constant-time primitives. Target-None handshake deliberately defers target selection; registry/routing rechecks and hosted config forbidding permissive shared mode remain separate pending files, not approved by this record.",
      "inspectedRanges": [
        [
          1,
          812
        ]
      ],
      "rationale": "MP-11: OK. Enforces signed scoped-token action/target/expiry/revocation admission. Read full verifier, decoder and claim validation: JWT requires HS256 and JWT type, signature covers header+claims using selected issuer HMAC, unknown actions reject, expiry and future issue checks fail closed on broken clock, live revocations are account/kind scoped, and comparisons use constant-time primitives. Target-None handshake deliberately defers target selection; registry/routing rechecks and hosted config forbidding permissive shared mode remain separate pending files, not approved by this record."
    }
  },
  {
    "id": "mp11narrow-91dfb7a22cf0c3d29ff8",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/relay/src/config.rs",
      "blob": "0697ab7ed089394655ac9e14e08f1bca8b1e958b",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "relay",
      "contextHash": "329615bbf4f48a1e16c6a92bd8fc60c233ec8d4a73574eea5cbffeadaca35dc7"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-91dfb7a22cf0c3d29ff8-review",
      "reviewedAt": "2026-10-05T05:38:42.891826+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Relay admission, scoped tokens, peer identity and protocol gating",
      "check": "Defines local/self-host relay bind configuration. Read the entire loader and validation: defaults bind loopback, zero port/empty host reject, optional shared token is explicitly legacy/local behavior. Debug derives include shared_token; no diagnostic call proving exposure was found in the inspected main-use seam, so this is not a confirmed disclosure finding. Hosted auth enforcement and all logging callers still require their own reviews.",
      "inspectedRanges": [
        [
          1,
          72
        ]
      ],
      "rationale": "MP-11: OK. Defines local/self-host relay bind configuration. Read the entire loader and validation: defaults bind loopback, zero port/empty host reject, optional shared token is explicitly legacy/local behavior. Debug derives include shared_token; no diagnostic call proving exposure was found in the inspected main-use seam, so this is not a confirmed disclosure finding. Hosted auth enforcement and all logging callers still require their own reviews."
    }
  },
  {
    "id": "mp11narrow-1b3cf645a05f7f3e045f",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/src/managed_bootstrap/release.rs",
      "blob": "abf3515163adcdeff679704e7680375d23b7f2ca",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "d0f7fcf52e4b1d3e78d1e3732149c7db9b60203e61f5fd771b4d1db58f0332ad"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-1b3cf645a05f7f3e045f-review",
      "reviewedAt": "2026-10-05T05:38:42.897490+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces signed manifest/kernel/build-attestation and active content-addressed path verification. Read all 674 lines: bounded regular-file checks, manifest hash before signature, unique artifact bindings, source/tree/target match, external builder pin equality when configured, consistent all-symlink versioned facades, exact service symlinks and retired service residue refusal. Root-controlled immutable release trees are a required caller assumption; no installed unit/drop-in, signing inventory, Rekor or deployment acceptance is claimed.",
      "inspectedRanges": [
        [
          1,
          674
        ]
      ],
      "rationale": "MP-11: OK. Enforces signed manifest/kernel/build-attestation and active content-addressed path verification. Read all 674 lines: bounded regular-file checks, manifest hash before signature, unique artifact bindings, source/tree/target match, external builder pin equality when configured, consistent all-symlink versioned facades, exact service symlinks and retired service residue refusal. Root-controlled immutable release trees are a required caller assumption; no installed unit/drop-in, signing inventory, Rekor or deployment acceptance is claimed."
    }
  },
  {
    "id": "mp11narrow-9c9528188f360ae96d01",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust.rs",
      "blob": "264c40c5f9bce97f519c9d80e5619af7a39d9cda",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "3731f32aff6aa9f2143f076cc9c985f00222362b7d12e0112b583f66f735c40a"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-9c9528188f360ae96d01-review",
      "reviewedAt": "2026-10-05T05:38:42.903040+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces owner-scoped publisher snapshots and writer commit fences. Read complete API: trusted snapshot fields are private, require_current demands the same SQLite transaction and rechecks owner/key/revision/revocation, decision mutation uses BEGIN IMMEDIATE with cancellation checks before and after writer admission, bounded identifiers and i64 conversions fail closed. Kernel human-consent authentication remains outside this persistence API and needs its own review.",
      "inspectedRanges": [
        [
          1,
          340
        ]
      ],
      "rationale": "MP-11: OK. Enforces owner-scoped publisher snapshots and writer commit fences. Read complete API: trusted snapshot fields are private, require_current demands the same SQLite transaction and rechecks owner/key/revision/revocation, decision mutation uses BEGIN IMMEDIATE with cancellation checks before and after writer admission, bounded identifiers and i64 conversions fail closed. Kernel human-consent authentication remains outside this persistence API and needs its own review."
    }
  },
  {
    "id": "mp11narrow-bc64ce3eded6c3dead63",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust.rs",
      "blob": "264c40c5f9bce97f519c9d80e5619af7a39d9cda",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "apps",
      "contextHash": "3731f32aff6aa9f2143f076cc9c985f00222362b7d12e0112b583f66f735c40a"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-bc64ce3eded6c3dead63-review",
      "reviewedAt": "2026-10-05T05:38:42.903082+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "App capability admission, publisher keys and host-action offers",
      "check": "Enforces owner-scoped publisher snapshots and writer commit fences. Read complete API: trusted snapshot fields are private, require_current demands the same SQLite transaction and rechecks owner/key/revision/revocation, decision mutation uses BEGIN IMMEDIATE with cancellation checks before and after writer admission, bounded identifiers and i64 conversions fail closed. Kernel human-consent authentication remains outside this persistence API and needs its own review.",
      "inspectedRanges": [
        [
          1,
          340
        ]
      ],
      "rationale": "MP-11: OK. Enforces owner-scoped publisher snapshots and writer commit fences. Read complete API: trusted snapshot fields are private, require_current demands the same SQLite transaction and rechecks owner/key/revision/revocation, decision mutation uses BEGIN IMMEDIATE with cancellation checks before and after writer admission, bounded identifiers and i64 conversions fail closed. Kernel human-consent authentication remains outside this persistence API and needs its own review."
    }
  },
  {
    "id": "mp11narrow-f717bc19637e14b61a7f",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust/enrollment.rs",
      "blob": "acb7f29266ef93c5e103d5a38be0314a036a0d71",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "0d1b407962a4eac35ca8d486564dac19b2921780be479fa2ba0264fd0797ce96"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-f717bc19637e14b61a7f-review",
      "reviewedAt": "2026-10-05T05:38:42.908567+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces atomic publisher enrollment within an existing writer transaction. Read whole helper: validates owner, identifiers, decision, timestamp/revision and rejects weak public keys; SAVEPOINT rolls back both key and receipt on error so callers cannot commit a partial enrollment. This API explicitly provides persistence, not authorization; authenticated kernel caller is still required.",
      "inspectedRanges": [
        [
          1,
          56
        ]
      ],
      "rationale": "MP-11: OK. Enforces atomic publisher enrollment within an existing writer transaction. Read whole helper: validates owner, identifiers, decision, timestamp/revision and rejects weak public keys; SAVEPOINT rolls back both key and receipt on error so callers cannot commit a partial enrollment. This API explicitly provides persistence, not authorization; authenticated kernel caller is still required."
    }
  },
  {
    "id": "mp11narrow-9bbd4d187d002a3cd3a0",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust/enrollment.rs",
      "blob": "acb7f29266ef93c5e103d5a38be0314a036a0d71",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "apps",
      "contextHash": "0d1b407962a4eac35ca8d486564dac19b2921780be479fa2ba0264fd0797ce96"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-9bbd4d187d002a3cd3a0-review",
      "reviewedAt": "2026-10-05T05:38:42.908589+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "App capability admission, publisher keys and host-action offers",
      "check": "Enforces atomic publisher enrollment within an existing writer transaction. Read whole helper: validates owner, identifiers, decision, timestamp/revision and rejects weak public keys; SAVEPOINT rolls back both key and receipt on error so callers cannot commit a partial enrollment. This API explicitly provides persistence, not authorization; authenticated kernel caller is still required.",
      "inspectedRanges": [
        [
          1,
          56
        ]
      ],
      "rationale": "MP-11: OK. Enforces atomic publisher enrollment within an existing writer transaction. Read whole helper: validates owner, identifiers, decision, timestamp/revision and rejects weak public keys; SAVEPOINT rolls back both key and receipt on error so callers cannot commit a partial enrollment. This API explicitly provides persistence, not authorization; authenticated kernel caller is still required."
    }
  },
  {
    "id": "mp11narrow-de2f8d5d2ddcc15529f9",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust/store.rs",
      "blob": "2ee0644fd7d01b84f5e916ecf588abb883d86d32",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "805b64b89089ea4b9ac176b06410b987157c15452981577e702873219a4fc49d"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-de2f8d5d2ddcc15529f9-review",
      "reviewedAt": "2026-10-05T05:38:42.913809+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces durable immutable owner/key binding, revision CAS and idempotent trust decisions. Read complete SQL and decode logic: all queries scope owner, corrupt weak keys reject, repeated exact decisions return historical receipts without reviving trust, conflicting key replacement rejects, receipt/key mutation share caller transaction, and admission reserves future revocation capacity. Writer transaction/consent are reviewed in the companion API, not inferred from receipt text.",
      "inspectedRanges": [
        [
          1,
          266
        ]
      ],
      "rationale": "MP-11: OK. Enforces durable immutable owner/key binding, revision CAS and idempotent trust decisions. Read complete SQL and decode logic: all queries scope owner, corrupt weak keys reject, repeated exact decisions return historical receipts without reviving trust, conflicting key replacement rejects, receipt/key mutation share caller transaction, and admission reserves future revocation capacity. Writer transaction/consent are reviewed in the companion API, not inferred from receipt text."
    }
  },
  {
    "id": "mp11narrow-17fc2b783387e0dd1761",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-runtime/src/publisher_trust/store.rs",
      "blob": "2ee0644fd7d01b84f5e916ecf588abb883d86d32",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "apps",
      "contextHash": "805b64b89089ea4b9ac176b06410b987157c15452981577e702873219a4fc49d"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-17fc2b783387e0dd1761-review",
      "reviewedAt": "2026-10-05T05:38:42.913843+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "App capability admission, publisher keys and host-action offers",
      "check": "Enforces durable immutable owner/key binding, revision CAS and idempotent trust decisions. Read complete SQL and decode logic: all queries scope owner, corrupt weak keys reject, repeated exact decisions return historical receipts without reviving trust, conflicting key replacement rejects, receipt/key mutation share caller transaction, and admission reserves future revocation capacity. Writer transaction/consent are reviewed in the companion API, not inferred from receipt text.",
      "inspectedRanges": [
        [
          1,
          266
        ]
      ],
      "rationale": "MP-11: OK. Enforces durable immutable owner/key binding, revision CAS and idempotent trust decisions. Read complete SQL and decode logic: all queries scope owner, corrupt weak keys reject, repeated exact decisions return historical receipts without reviving trust, conflicting key replacement rejects, receipt/key mutation share caller transaction, and admission reserves future revocation capacity. Writer transaction/consent are reviewed in the companion API, not inferred from receipt text."
    }
  },
  {
    "id": "mp11narrow-a5725ce6e77928e3f443",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-package/src/developer/keys.rs",
      "blob": "138fc80fd007a782b297a629c7727d0e7c17656a",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "4194b27f2c20beee010e55c7f413ea3cf266210134d6e2cdf50e88bc06a3c1ca"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-a5725ce6e77928e3f443-review",
      "reviewedAt": "2026-10-05T05:38:42.919206+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces developer publisher key generation and public enrollment validation. Read full file: entropy fills a zeroizing Ed25519 seed; enrollment binds canonical base64, fingerprint-derived key id and nonweak public key; PreparedFile publishes the private key before public material and refuses aliased destinations; key read requires exactly 32 bytes via private fs API. fs descriptor/path policy remains a separate pending dependency. This file does not enroll trust or prove off-device backup/approved fingerprint.",
      "inspectedRanges": [
        [
          1,
          166
        ]
      ],
      "rationale": "MP-11: OK. Enforces developer publisher key generation and public enrollment validation. Read full file: entropy fills a zeroizing Ed25519 seed; enrollment binds canonical base64, fingerprint-derived key id and nonweak public key; PreparedFile publishes the private key before public material and refuses aliased destinations; key read requires exactly 32 bytes via private fs API. fs descriptor/path policy remains a separate pending dependency. This file does not enroll trust or prove off-device backup/approved fingerprint."
    }
  },
  {
    "id": "mp11narrow-93ab7cadb7ef46610ccd",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "packages/app-package/src/developer/keys.rs",
      "blob": "138fc80fd007a782b297a629c7727d0e7c17656a",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "apps",
      "contextHash": "4194b27f2c20beee010e55c7f413ea3cf266210134d6e2cdf50e88bc06a3c1ca"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-93ab7cadb7ef46610ccd-review",
      "reviewedAt": "2026-10-05T05:38:42.919230+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "App capability admission, publisher keys and host-action offers",
      "check": "Enforces developer publisher key generation and public enrollment validation. Read full file: entropy fills a zeroizing Ed25519 seed; enrollment binds canonical base64, fingerprint-derived key id and nonweak public key; PreparedFile publishes the private key before public material and refuses aliased destinations; key read requires exactly 32 bytes via private fs API. fs descriptor/path policy remains a separate pending dependency. This file does not enroll trust or prove off-device backup/approved fingerprint.",
      "inspectedRanges": [
        [
          1,
          166
        ]
      ],
      "rationale": "MP-11: OK. Enforces developer publisher key generation and public enrollment validation. Read full file: entropy fills a zeroizing Ed25519 seed; enrollment binds canonical base64, fingerprint-derived key id and nonweak public key; PreparedFile publishes the private key before public material and refuses aliased destinations; key read requires exactly 32 bytes via private fs API. fs descriptor/path policy remains a separate pending dependency. This file does not enroll trust or prove off-device backup/approved fingerprint."
    }
  },
  {
    "id": "mp11narrow-a729c3b8c50e5abfd4f1",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "scripts/sign-app-runtime-release.mjs",
      "blob": "a52e1795109b4c24310333ed1352d3a0e89b9992",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "1a5a88f98a5064d80e1d2eb084c8dc1589bd645e8bd09a48a7d9899e716942af"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-a729c3b8c50e5abfd4f1-review",
      "reviewedAt": "2026-10-05T05:38:42.925015+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces offline runtime signing against builder attestation and source identity. Read entire script: checks current source build inputs and launcher hashes, signed builder proof, exact artifact set, private owned Ed25519 signing key, signature-only macOS changes, copied byte digests and inventory caps; signs immutable inventory and returns public key only. Cleanup checks output inode/device and never installs trust. Imported attestation/fs/codesign helpers require separate review; no private key or artifact was read/executed in this lane.",
      "inspectedRanges": [
        [
          1,
          169
        ]
      ],
      "rationale": "MP-11: OK. Enforces offline runtime signing against builder attestation and source identity. Read entire script: checks current source build inputs and launcher hashes, signed builder proof, exact artifact set, private owned Ed25519 signing key, signature-only macOS changes, copied byte digests and inventory caps; signs immutable inventory and returns public key only. Cleanup checks output inode/device and never installs trust. Imported attestation/fs/codesign helpers require separate review; no private key or artifact was read/executed in this lane."
    }
  },
  {
    "id": "mp11narrow-b983970df0cabd0ba1f9",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/cli/src/app-dev-loop.ts",
      "blob": "4dabf79787eee675bcce0db71e73d44dfaad2b34",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "apps",
      "contextHash": "2e368819d73fd5e12f5cbb4942f384a1f885671b12fdf7e7769a1cf007a4ec48"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-b983970df0cabd0ba1f9-review",
      "reviewedAt": "2026-10-05T05:38:42.930882+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "App capability admission, publisher keys and host-action offers",
      "check": "Projects developer key selection and App install/update lifecycle. Read complete file: current default is ~/.chariox/keys/app-publisher/private, guidance preserves existing keys and requires protected storage/fingerprint/backup, temporary package workspace is outside App source, and changed capabilities retain kernel approval flow. This resolves the old B211-KEY source observation at this blob; live parity disposition belongs to other lanes. No signing key was loaded.",
      "inspectedRanges": [
        [
          1,
          286
        ]
      ],
      "rationale": "MP-11: OK. Projects developer key selection and App install/update lifecycle. Read complete file: current default is ~/.chariox/keys/app-publisher/private, guidance preserves existing keys and requires protected storage/fingerprint/backup, temporary package workspace is outside App source, and changed capabilities retain kernel approval flow. This resolves the old B211-KEY source observation at this blob; live parity disposition belongs to other lanes. No signing key was loaded."
    }
  },
  {
    "id": "mp11narrow-33da1fc8d9f5f5c2d75c",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-authority.mjs",
      "blob": "5fc73e66da8a0184e93529c27fd0f8595d597ed0",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "d35520b9cdc7e22c7a52e79946d197a603fb4da74a95cae3bd7f9d4cd927cfa0"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-33da1fc8d9f5f5c2d75c-review",
      "reviewedAt": "2026-10-05T05:38:42.936701+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Selects attested managed-rootless or explicit local DEV broker authority. Read complete helper: local override requires root, numeric owner enrollment, private barrier/installed source check, exact helper-name match and one inspected helper, fixed rootful socket/engine with pinned launch socket identity and namespace maps. Imported enrollment/topology/namespace verifiers remain separately pending; no product runtime authority was invoked.",
      "inspectedRanges": [
        [
          1,
          38
        ]
      ],
      "rationale": "MP-11: OK. Selects attested managed-rootless or explicit local DEV broker authority. Read complete helper: local override requires root, numeric owner enrollment, private barrier/installed source check, exact helper-name match and one inspected helper, fixed rootful socket/engine with pinned launch socket identity and namespace maps. Imported enrollment/topology/namespace verifiers remain separately pending; no product runtime authority was invoked."
    }
  },
  {
    "id": "mp11narrow-8a0130b936c460b35adf",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-layout.mjs",
      "blob": "f8b5e8ff9eee553d497bc467422d94c8ce3e82bc",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signing",
      "contextHash": "6babd3f69d9044387a98ede585895acb50b990f0f9fe04f6f314be2931ceef90"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-8a0130b936c460b35adf-review",
      "reviewedAt": "2026-10-05T05:38:42.941928+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Release/image signing, trust pins, cosign, Rekor and receipt verification",
      "check": "Enforces separation of captured public home from private kernel/provider identity. Read all layout guards: trusted base image/container binding, normalized absolute mounts, exact private/home/NSS mounts, runtime ancestor/descendant mount refusal, duplicate/unallowlisted environment rejection, fixed private env and HOME, owner-bound volume names, and traversal/known credential roots refused. Arbitrarily named user secrets are outside this declared policy; broker receipt/ownership checks are separate dependencies.",
      "inspectedRanges": [
        [
          1,
          86
        ]
      ],
      "rationale": "MP-11: OK. Enforces separation of captured public home from private kernel/provider identity. Read all layout guards: trusted base image/container binding, normalized absolute mounts, exact private/home/NSS mounts, runtime ancestor/descendant mount refusal, duplicate/unallowlisted environment rejection, fixed private env and HOME, owner-bound volume names, and traversal/known credential roots refused. Arbitrarily named user secrets are outside this declared policy; broker receipt/ownership checks are separate dependencies."
    }
  },
  {
    "id": "mp11narrow-bcdc666b4d4b43cf20b1",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-layout.mjs",
      "blob": "f8b5e8ff9eee553d497bc467422d94c8ce3e82bc",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "6babd3f69d9044387a98ede585895acb50b990f0f9fe04f6f314be2931ceef90"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-bcdc666b4d4b43cf20b1-review",
      "reviewedAt": "2026-10-05T05:38:42.941959+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces separation of captured public home from private kernel/provider identity. Read all layout guards: trusted base image/container binding, normalized absolute mounts, exact private/home/NSS mounts, runtime ancestor/descendant mount refusal, duplicate/unallowlisted environment rejection, fixed private env and HOME, owner-bound volume names, and traversal/known credential roots refused. Arbitrarily named user secrets are outside this declared policy; broker receipt/ownership checks are separate dependencies.",
      "inspectedRanges": [
        [
          1,
          86
        ]
      ],
      "rationale": "MP-11: OK. Enforces separation of captured public home from private kernel/provider identity. Read all layout guards: trusted base image/container binding, normalized absolute mounts, exact private/home/NSS mounts, runtime ancestor/descendant mount refusal, duplicate/unallowlisted environment rejection, fixed private env and HOME, owner-bound volume names, and traversal/known credential roots refused. Arbitrarily named user secrets are outside this declared policy; broker receipt/ownership checks are separate dependencies."
    }
  },
  {
    "id": "mp11narrow-9b718688abb5bb051293",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-host-root.mjs",
      "blob": "acf2d2f89700e45362fd39a3f765d53115bb42f1",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "9c76ae34a760c17e11227c69ebc2a30d9abaca77625564db5ee2d2c0a68f5a78"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-9b718688abb5bb051293-review",
      "reviewedAt": "2026-10-05T05:38:42.947679+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces durable private host-root ownership and no silent identity regeneration. Read complete helper: realpath equality, lstat on every component, directory/non-symlink/non-writable ancestor checks and exact leaf modes; fresh creation is nonrecursive and exclusive, retained marker version/slice/owner/dataOwner must match. Runtime identity existence is deferred to provisioner by design. Namespace-approved ancestor verification remains separate.",
      "inspectedRanges": [
        [
          1,
          53
        ]
      ],
      "rationale": "MP-11: OK. Enforces durable private host-root ownership and no silent identity regeneration. Read complete helper: realpath equality, lstat on every component, directory/non-symlink/non-writable ancestor checks and exact leaf modes; fresh creation is nonrecursive and exclusive, retained marker version/slice/owner/dataOwner must match. Runtime identity existence is deferred to provisioner by design. Namespace-approved ancestor verification remains separate."
    }
  },
  {
    "id": "mp11narrow-36f6c29a75fc63abc391",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-rootless-owner.mjs",
      "blob": "14e6228becc3822086039419d7108c266ad1ca62",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "a07c9e05babde8b9293272c15d83dbd8909a6fc34e03ba134a7659ce79143517"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-36f6c29a75fc63abc391-review",
      "reviewedAt": "2026-10-05T05:38:42.953394+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces selecting slice data ownership from the verified daemon namespace rather than host subordinate IDs. Read complete nine-line wrapper and its exported owner source; it delegates to readNamespaceEntry and returns dataUid. Namespace proof enforcement is a separate pending blob; this record approves the wiring only.",
      "inspectedRanges": [
        [
          1,
          9
        ]
      ],
      "rationale": "MP-11: OK. Enforces selecting slice data ownership from the verified daemon namespace rather than host subordinate IDs. Read complete nine-line wrapper and its exported owner source; it delegates to readNamespaceEntry and returns dataUid. Namespace proof enforcement is a separate pending blob; this record approves the wiring only."
    }
  },
  {
    "id": "mp11narrow-d7fe06a93a6918a4fedf",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-layout-store.mjs",
      "blob": "830cb105a5d6912238a589ef7dae3968c7ad8709",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "eb88abfddcd0db771d50f37c0ee4771458b5e69018c3f14e2693e3f4fedfabbb"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-d7fe06a93a6918a4fedf-review",
      "reviewedAt": "2026-10-05T05:38:42.959388+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces private public-metadata receipts and retained identity-file presence. Read complete code: private directory and identifiers, exclusive nofollow 0600 staging, fsync/atomic rename and parent sync; bounded single-link owner/private receipt reads; identity paths limited to kernel identities/registry without traversal and private directories/files required. Path lstat after descriptor open relies on an already owner-private directory; privileged owner compromise is outside this trust boundary.",
      "inspectedRanges": [
        [
          1,
          48
        ]
      ],
      "rationale": "MP-11: OK. Enforces private public-metadata receipts and retained identity-file presence. Read complete code: private directory and identifiers, exclusive nofollow 0600 staging, fsync/atomic rename and parent sync; bounded single-link owner/private receipt reads; identity paths limited to kernel identities/registry without traversal and private directories/files required. Path lstat after descriptor open relies on an already owner-private directory; privileged owner compromise is outside this trust boundary."
    }
  },
  {
    "id": "mp11narrow-a26e89f06447ad30896a",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-runtime-proof.mjs",
      "blob": "a90a15fb89431f3173c65bdbc99e30ed3205f2e0",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "a5a2dad178ec54eba63b8dd4ec0a6ee7a6f41482d0d2da574d4e2e4e7350a798"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-a26e89f06447ad30896a-review",
      "reviewedAt": "2026-10-05T05:38:42.965413+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces exact kernel executable hash and immutable root-owned runtime ancestors. Read complete helper: expected hash shape, exactly five stat rows, exact paths/types/uid and no group/world write, fixed root Docker stat/hash commands and hash equality. Trusted engine/root admin is explicit; receipt source trust and later runtime lifecycle remain separate pending anchors.",
      "inspectedRanges": [
        [
          1,
          32
        ]
      ],
      "rationale": "MP-11: OK. Enforces exact kernel executable hash and immutable root-owned runtime ancestors. Read complete helper: expected hash shape, exactly five stat rows, exact paths/types/uid and no group/world write, fixed root Docker stat/hash commands and hash equality. Trusted engine/root admin is explicit; receipt source trust and later runtime lifecycle remain separate pending anchors."
    }
  },
  {
    "id": "mp11narrow-0b9e510665dc90265157",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-image-proof.mjs",
      "blob": "434782bfcff58100c3e3f3a9f7c27233da88517e",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "837bb9086927a631832d789834dfeea493a8bf0e20721bf35ab2197d0467ba91"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-0b9e510665dc90265157-review",
      "reviewedAt": "2026-10-05T05:38:42.971164+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces image lineage proofs from standard build or broker capture/flattening. Read full script: immutable sha256 identities, source digest, slice user and ordered valid layers, inherited kernel hash, parent/container/environment equality, strict one-layer flatten proof; isolated no-network/read-only bounded hash probe and exact randomly named helper cleanup. Callers must establish successful trusted build/capture before minting; this module does not authenticate caller-supplied sourceDigest itself.",
      "inspectedRanges": [
        [
          1,
          103
        ]
      ],
      "rationale": "MP-11: OK. Enforces image lineage proofs from standard build or broker capture/flattening. Read full script: immutable sha256 identities, source digest, slice user and ordered valid layers, inherited kernel hash, parent/container/environment equality, strict one-layer flatten proof; isolated no-network/read-only bounded hash probe and exact randomly named helper cleanup. Callers must establish successful trusted build/capture before minting; this module does not authenticate caller-supplied sourceDigest itself."
    }
  },
  {
    "id": "mp11narrow-142cf09c95ec08030ea0",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-preflight.mjs",
      "blob": "1be4d96b877bd05206da538c4ba31fba3f0a667e",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "6effdf8a84c107490787cc566cab0eb62d1f5bf105201d14cf2a75f6fcc0a8cf"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-142cf09c95ec08030ea0-review",
      "reviewedAt": "2026-10-05T05:38:42.976557+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces home-volume capture admission. Read full bounded walk: exact Docker volume identity and expected canonical mountpoint, nonsymlink directory, entry/deadline cap, no setuid/setgid, known credential-root refusal, and strict symlink/type validation only after quiescence. Caller-proven writers quiescence is required to prevent walk races; no archive or volume was inspected here.",
      "inspectedRanges": [
        [
          1,
          36
        ]
      ],
      "rationale": "MP-11: OK. Enforces home-volume capture admission. Read full bounded walk: exact Docker volume identity and expected canonical mountpoint, nonsymlink directory, entry/deadline cap, no setuid/setgid, known credential-root refusal, and strict symlink/type validation only after quiescence. Caller-proven writers quiescence is required to prevent walk races; no archive or volume was inspected here."
    }
  },
  {
    "id": "mp11narrow-78bb88cb5e12e85f1150",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-retirement.mjs",
      "blob": "fdb8bb212c108c588914d6a2e17176ba1f3588e9",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "b9df7a3bce767cab5e3ee85b1b181af5288929444da8991c86a4004f3ab28f6f"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-78bb88cb5e12e85f1150-review",
      "reviewedAt": "2026-10-05T05:38:42.981919+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces retiring only retained quota homes proven to belong to this slice/kernel/machine. Read full helper: quota identity validation precedes inspect, only exact not-found is idempotent, local driver/name/owner labels required, and in-use remove failure preserves reservation. Container engine authority and quota identity helper remain separately pending.",
      "inspectedRanges": [
        [
          1,
          24
        ]
      ],
      "rationale": "MP-11: OK. Enforces retiring only retained quota homes proven to belong to this slice/kernel/machine. Read full helper: quota identity validation precedes inspect, only exact not-found is idempotent, local driver/name/owner labels required, and in-use remove failure preserves reservation. Container engine authority and quota identity helper remain separately pending."
    }
  },
  {
    "id": "mp11narrow-78703c9e94eec5781d22",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-local-home-scan.mjs",
      "blob": "109d24dcc66ea0e9f77f110d3e0d4f090ea7b394",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "c92e0414071a197384ef8f5fea64a2d400c324cea6b4fe5cabb4f9fe24f56bfd"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-78703c9e94eec5781d22-review",
      "reviewedAt": "2026-10-05T05:38:42.987317+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces local home inventory and isolated helper ownership. Read complete code: NUL field counts and bounded entry count, protected roots and setid modes reject, optional strict link checks after quiescence; exact local volume driver/scope/options; unique helper mounts only source readonly with network none, dropped capabilities and no-new-privileges plus bounded resources. Proven worker image/enrollment and stop/quiescence are caller assumptions.",
      "inspectedRanges": [
        [
          1,
          39
        ]
      ],
      "rationale": "MP-11: OK. Enforces local home inventory and isolated helper ownership. Read complete code: NUL field counts and bounded entry count, protected roots and setid modes reject, optional strict link checks after quiescence; exact local volume driver/scope/options; unique helper mounts only source readonly with network none, dropped capabilities and no-new-privileges plus bounded resources. Proven worker image/enrollment and stop/quiescence are caller assumptions."
    }
  },
  {
    "id": "mp11narrow-b6be125b27add4904602",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-capture.mjs",
      "blob": "c83f0433348fa53c8848f78cd9d3724a465cec28",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "ddbb7a417c16cd34723f93ec4555eaccae713492af304c6787c7a017741fc7c4"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-b6be125b27add4904602-review",
      "reviewedAt": "2026-10-05T05:38:42.992698+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces owned snapshot-helper topology and capture safety. Read full file: helper/volume owner name, private sink directory, exact snapshot-helper label/no network/single readonly home mount; preflight known credential/type/link refusal then protected sink streaming and post-capture archive validation. Failure and signals remove only verified exact helper; completed bad archive is removed. Explicit Release F legacy skips modern safety checks, so legacy admission remains separately pending and is not blanket-approved.",
      "inspectedRanges": [
        [
          1,
          90
        ]
      ],
      "rationale": "MP-11: OK. Enforces owned snapshot-helper topology and capture safety. Read full file: helper/volume owner name, private sink directory, exact snapshot-helper label/no network/single readonly home mount; preflight known credential/type/link refusal then protected sink streaming and post-capture archive validation. Failure and signals remove only verified exact helper; completed bad archive is removed. Explicit Release F legacy skips modern safety checks, so legacy admission remains separately pending and is not blanket-approved."
    }
  },
  {
    "id": "mp11narrow-860efc6581009d32a6c8",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-capture.mjs",
      "blob": "c83f0433348fa53c8848f78cd9d3724a465cec28",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signals",
      "contextHash": "ddbb7a417c16cd34723f93ec4555eaccae713492af304c6787c7a017741fc7c4"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-860efc6581009d32a6c8-review",
      "reviewedAt": "2026-10-05T05:38:42.992728+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Process signal and kill guards, including drill cleanup",
      "check": "Enforces owned snapshot-helper topology and capture safety. Read full file: helper/volume owner name, private sink directory, exact snapshot-helper label/no network/single readonly home mount; preflight known credential/type/link refusal then protected sink streaming and post-capture archive validation. Failure and signals remove only verified exact helper; completed bad archive is removed. Explicit Release F legacy skips modern safety checks, so legacy admission remains separately pending and is not blanket-approved.",
      "inspectedRanges": [
        [
          1,
          90
        ]
      ],
      "rationale": "MP-11: OK. Enforces owned snapshot-helper topology and capture safety. Read full file: helper/volume owner name, private sink directory, exact snapshot-helper label/no network/single readonly home mount; preflight known credential/type/link refusal then protected sink streaming and post-capture archive validation. Failure and signals remove only verified exact helper; completed bad archive is removed. Explicit Release F legacy skips modern safety checks, so legacy admission remains separately pending and is not blanket-approved."
    }
  },
  {
    "id": "mp11narrow-78464ec14b155993b08f",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-archive-stream.mjs",
      "blob": "cd5d168384848641bbc0e2ea04e0173533639acb",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "6b339e35dfc42948605022924e9da5e0b0401c53c621b50da7448625c014c865"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-78464ec14b155993b08f-review",
      "reviewedAt": "2026-10-05T05:38:42.998146+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces exclusive private archive sink and bounded capture lifetime/storage. Read full stream lifecycle: owner-private directory, O_EXCL/O_NOFOLLOW 0600 destination, capacity/size watchdog, child handle TERM/KILL escalation, timers cleared after close, private regular single-link metadata and fsync/hash before success. Numeric process-group APIs are absent; direct spawned ChildProcess handles preserve ownership. Trusted parent directory prevents path swap by untrusted users; archive content validation is downstream.",
      "inspectedRanges": [
        [
          1,
          54
        ]
      ],
      "rationale": "MP-11: OK. Enforces exclusive private archive sink and bounded capture lifetime/storage. Read full stream lifecycle: owner-private directory, O_EXCL/O_NOFOLLOW 0600 destination, capacity/size watchdog, child handle TERM/KILL escalation, timers cleared after close, private regular single-link metadata and fsync/hash before success. Numeric process-group APIs are absent; direct spawned ChildProcess handles preserve ownership. Trusted parent directory prevents path swap by untrusted users; archive content validation is downstream."
    }
  },
  {
    "id": "mp11narrow-f350f85f346f52057e80",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-archive-stream.mjs",
      "blob": "cd5d168384848641bbc0e2ea04e0173533639acb",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signals",
      "contextHash": "6b339e35dfc42948605022924e9da5e0b0401c53c621b50da7448625c014c865"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-f350f85f346f52057e80-review",
      "reviewedAt": "2026-10-05T05:38:42.998166+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Process signal and kill guards, including drill cleanup",
      "check": "Enforces exclusive private archive sink and bounded capture lifetime/storage. Read full stream lifecycle: owner-private directory, O_EXCL/O_NOFOLLOW 0600 destination, capacity/size watchdog, child handle TERM/KILL escalation, timers cleared after close, private regular single-link metadata and fsync/hash before success. Numeric process-group APIs are absent; direct spawned ChildProcess handles preserve ownership. Trusted parent directory prevents path swap by untrusted users; archive content validation is downstream.",
      "inspectedRanges": [
        [
          1,
          54
        ]
      ],
      "rationale": "MP-11: OK. Enforces exclusive private archive sink and bounded capture lifetime/storage. Read full stream lifecycle: owner-private directory, O_EXCL/O_NOFOLLOW 0600 destination, capacity/size watchdog, child handle TERM/KILL escalation, timers cleared after close, private regular single-link metadata and fsync/hash before success. Numeric process-group APIs are absent; direct spawned ChildProcess handles preserve ownership. Trusted parent directory prevents path swap by untrusted users; archive content validation is downstream."
    }
  },
  {
    "id": "mp11narrow-4f594e9f8735177308e0",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-restore.mjs",
      "blob": "0a7d3caf60e2e4fe8c7ba41c23006893c3fbd71b",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "sandbox",
      "contextHash": "4b5acfeaa76a396d2771c92a413d593cfc2eebb15cd59fa06c0884c4db2986d8"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-4f594e9f8735177308e0-review",
      "reviewedAt": "2026-10-05T05:38:43.003648+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Sandbox, namespace, seccomp, AppArmor, cgroup and protected filesystem setup",
      "check": "Enforces restoring a pinned private archive only into an empty owned generation. Read entire module: proc-fd pin checks, fresh owned single-mount helper, validation before extraction, bounded streaming and reserve/deadline cancellation, exact helper abort, chown/sync before generation completion; child handles are owned spawned processes. Archive validator and generation-store policy remain separate pending source anchors. No restore run, secret archive read, or Selkies-specific work occurred.",
      "inspectedRanges": [
        [
          1,
          186
        ]
      ],
      "rationale": "MP-11: OK. Enforces restoring a pinned private archive only into an empty owned generation. Read entire module: proc-fd pin checks, fresh owned single-mount helper, validation before extraction, bounded streaming and reserve/deadline cancellation, exact helper abort, chown/sync before generation completion; child handles are owned spawned processes. Archive validator and generation-store policy remain separate pending source anchors. No restore run, secret archive read, or Selkies-specific work occurred."
    }
  },
  {
    "id": "mp11narrow-7aa36f0c717817ec4da3",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/protected-home-restore.mjs",
      "blob": "0a7d3caf60e2e4fe8c7ba41c23006893c3fbd71b",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signals",
      "contextHash": "4b5acfeaa76a396d2771c92a413d593cfc2eebb15cd59fa06c0884c4db2986d8"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-7aa36f0c717817ec4da3-review",
      "reviewedAt": "2026-10-05T05:38:43.003685+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Process signal and kill guards, including drill cleanup",
      "check": "Enforces restoring a pinned private archive only into an empty owned generation. Read entire module: proc-fd pin checks, fresh owned single-mount helper, validation before extraction, bounded streaming and reserve/deadline cancellation, exact helper abort, chown/sync before generation completion; child handles are owned spawned processes. Archive validator and generation-store policy remain separate pending source anchors. No restore run, secret archive read, or Selkies-specific work occurred.",
      "inspectedRanges": [
        [
          1,
          186
        ]
      ],
      "rationale": "MP-11: OK. Enforces restoring a pinned private archive only into an empty owned generation. Read entire module: proc-fd pin checks, fresh owned single-mount helper, validation before extraction, bounded streaming and reserve/deadline cancellation, exact helper abort, chown/sync before generation completion; child handles are owned spawned processes. Archive validator and generation-store policy remain separate pending source anchors. No restore run, secret archive read, or Selkies-specific work occurred."
    }
  },
  {
    "id": "mp11narrow-24612addd568c2e1e469",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/docker/browser-controller-cookie-fence.mjs",
      "blob": "3bc11f0ac45f9e75bd3c022216d3e64ac20a642d",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "browser",
      "contextHash": "4d4fa4f47c8468626713ac559dab96f207d42eaebd1ade47cc4c82558e50f006"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-24612addd568c2e1e469-review",
      "reviewedAt": "2026-10-05T05:38:43.008919+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Protected frames, Vault masking and isolated-world observation capture",
      "check": "Enforces freezing browser cookie writers across pages/workers for mutation isolation. Read full module: subscribe before autoattach, pause known writers, disable script/network and stop workers/loading, require network-idle/target-gone guards, close untracked writer targets, freeze pages, and unwind all writer/page/debugger states on failure/release with recoveryRequired. CDP session ownership and caller settlement are pending dependency reviews; no actual browser was run.",
      "inspectedRanges": [
        [
          1,
          202
        ]
      ],
      "rationale": "MP-11: OK. Enforces freezing browser cookie writers across pages/workers for mutation isolation. Read full module: subscribe before autoattach, pause known writers, disable script/network and stop workers/loading, require network-idle/target-gone guards, close untracked writer targets, freeze pages, and unwind all writer/page/debugger states on failure/release with recoveryRequired. CDP session ownership and caller settlement are pending dependency reviews; no actual browser was run."
    }
  },
  {
    "id": "mp11narrow-119c6f8f79eb590710d9",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/kernel/slice-linux-docker/docker/browser-controller-frames.mjs",
      "blob": "a1c83c92683d1a252b2488ceda47b35ff3307d53",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "browser",
      "contextHash": "975682446e0ec1e76ba4210ac84a533d0b78740dd6348312fa25ef68928a2807"
    },
    "disposition": "ordinary_path1_behavior",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-119c6f8f79eb590710d9-review",
      "reviewedAt": "2026-10-05T05:38:43.014498+00:00",
      "verdict": "OK",
      "severity": null,
      "enforces": "Protected frames, Vault masking and isolated-world observation capture",
      "check": "Enforces frame/document identity and owned isolated-renderer references. Read full module: frame/session bounds, resume/detach paused children on overflow/failure/removal, select descendants of the top tree, namespace backend references by frame+loader, bound merged snapshots and recheck tree identity, recheck ancestor identities before actions, and detach temporary sessions in finally. Protected screenshot masking/isolated-world collector in snapshot.mjs is separately pending; this is frame ownership review only.",
      "inspectedRanges": [
        [
          1,
          256
        ]
      ],
      "rationale": "MP-11: OK. Enforces frame/document identity and owned isolated-renderer references. Read full module: frame/session bounds, resume/detach paused children on overflow/failure/removal, select descendants of the top tree, namespace backend references by frame+loader, bound merged snapshots and recheck tree identity, recheck ancestor identities before actions, and detach temporary sessions in finally. Protected screenshot masking/isolated-world collector in snapshot.mjs is separately pending; this is frame ownership review only."
    }
  },
  {
    "id": "mp11narrow-d2f0453a573ab5d85714",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "apps/cli/scripts/lib/managed-browser-computer-parity-host-observer.mjs",
      "blob": "0d0bd31fe3cb97bce715500ff1ef5aa264a61d6d",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signals",
      "contextHash": "dd27f0df7fdea7a88a1d8db21577fca77b905d87b066f3b9635c49d892759704"
    },
    "disposition": "removal_required",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-d2f0453a573ab5d85714-review",
      "reviewedAt": "2026-10-05T05:38:43.020878+00:00",
      "verdict": "FINDING",
      "severity": "P2",
      "enforces": "Process signal and kill guards, including drill cleanup",
      "check": "Enforces bounded SSH host observation but fails the current process-group guard requirement. Read full module: host/engine inputs and output/deadline caps are bounded, but killGroup at lines 40-42 accepts any truthy child.pid and calls process.kill(-child.pid); close calls it after leader exit without checking group membership/liveness. F2 proposes shared owned-group verification rejecting <=1, nonintegers/NaN/undefined and verifying every member belongs to this run before each signal. No process or remote host was contacted; no wrong-group signal was demonstrated.",
      "inspectedRanges": [
        [
          1,
          79
        ]
      ],
      "rationale": "MP-11: FINDING. P2. Enforces bounded SSH host observation but fails the current process-group guard requirement. Read full module: host/engine inputs and output/deadline caps are bounded, but killGroup at lines 40-42 accepts any truthy child.pid and calls process.kill(-child.pid); close calls it after leader exit without checking group membership/liveness. F2 proposes shared owned-group verification rejecting <=1, nonintegers/NaN/undefined and verifying every member belongs to this run before each signal. No process or remote host was contacted; no wrong-group signal was demonstrated."
    }
  },
  {
    "id": "mp11narrow-890db6678f77cea57b3c",
    "sourceCommit": "74e50b787a5919ee5c3d580c5b088989fd4a1adf",
    "sourceTree": "3cb4ef5c883e8d358d0beab455797ded180a2406",
    "anchor": {
      "path": "scripts/test-app-storage-macos-ci.mjs",
      "blob": "021e99867296fc61422d2f54afce5ab68df922b5",
      "line": 1,
      "column": 1,
      "symbol": null,
      "category": "security_critical",
      "selector": "signals",
      "contextHash": "97462f23f38319680f1fd8fb26b4c41afdda0897d1c6835ddb12fe7bfb216d9e"
    },
    "disposition": "removal_required",
    "independentReview": {
      "independent": true,
      "reviewer": "GPT-6.1-sol (Codex), mp11narrow source lane; runtime non-author",
      "reviewId": "mp11narrow-890db6678f77cea57b3c-review",
      "reviewedAt": "2026-10-05T05:38:43.026881+00:00",
      "verdict": "FINDING",
      "severity": "P2",
      "enforces": "Process signal and kill guards, including drill cleanup",
      "check": "Enforces dedicated hosted CI storage-drill resource constraints but has an unguarded raw process-group kill. Read entire wrapper: isolated temp/evidence/build directories, bounded logs and resource watcher, and owned-root inode cleanup are present. stop at line 50 calls process.kill(-child.pid) without an explicit safe integer >1/existing owned-group check. F2 proposes the shared guarded group helper and injected invalid/group-reuse regressions. This macOS-only script was not run; no Rust invocation or signal occurred.",
      "inspectedRanges": [
        [
          1,
          119
        ]
      ],
      "rationale": "MP-11: FINDING. P2. Enforces dedicated hosted CI storage-drill resource constraints but has an unguarded raw process-group kill. Read entire wrapper: isolated temp/evidence/build directories, bounded logs and resource watcher, and owned-root inode cleanup are present. stop at line 50 calls process.kill(-child.pid) without an explicit safe integer >1/existing owned-group check. F2 proposes the shared guarded group helper and injected invalid/group-reuse regressions. This macOS-only script was not run; no Rust invocation or signal occurred."
    }
  }
]);
