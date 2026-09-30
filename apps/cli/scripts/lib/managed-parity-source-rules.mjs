// Scoped source audit rules are provisional observations, never independent
// semantic approvals. Each rule binds an inspected blob; changed blobs retain
// candidates but lose the classification until re-inspected.
const OSS = "b375eb6eec1a6700c329830fd3bfba581eb818dc";
const OSS_RUNTIME = "0e250d53977a49a73c3c3ee0f9251d9f9febe712";
const CLOUD = "a8f5ee1bc80f13f508cf950752d779399df1aa55";
const CLOUD_CURRENT = "8e9c24e4be0e87062343f60cda66f4c153539f91";
export const SOURCE_AUDIT_RULES = Object.freeze([
  {
    id: "managed-auto-stop-deadline", sourceCommit: CLOUD_CURRENT,
    path: "apps/api/src/managed-environments/auto-stop-deadline.ts",
    blob: "7c7b41da124ff1e8b7ef0ee67e0c19b834be4c45",
    classification: "mandatory_managed_shutdown_control",
    rationale: "The managed lifecycle validates configured durations and returns no idle deadline when idle stopping is disabled. An active deadline is the maximum of minimum runtime, delay since the idle transition and the shutdown warning interval. This is the permitted shutdown policy, not a provider/runtime restriction. Live timing and trigger coverage remain open.",
    anchors: [["automatic_shutdown_selector", "normalizeAutoStopPolicy"], ["automatic_shutdown_selector", "managedEnvironmentIdleDeadline"]],
  },
  {
    id: "managed-activity-and-enrollment-cleanup", sourceCommit: CLOUD_CURRENT,
    path: "apps/api/src/managed-environments/activity-service.ts",
    blob: "60da3c3e037c491fe9a1f8c7a16973f285c1d600",
    classification: "managed_shutdown_and_enrollment_lifecycle",
    rationale: "Signed activity reports require the active exact machine and confirmed kernel, preserve monotonic sequence/transition time and retain an unchanged idle deadline. Due ready machines request quiescence; expired enrollment cleanup requires the current revision's terminal nonretryable create operation and atomically creates an idempotent delete operation. These are shutdown/deployment lifecycle controls. This file does not establish downstream quiescence, provider stop execution or fresh-machine coverage.",
    anchors: [["automatic_shutdown_selector", "createManagedEnvironmentActivityService"], ["automatic_shutdown_selector", "validateReport"], ["automatic_shutdown_selector", "activitySignature"], ["automatic_shutdown_selector", "activityDeadline"], ["automatic_shutdown_selector", "activityResult"], ["release_activation", "requestDueEnrollmentCleanup"]],
  },
  {
    id: "release-update-outcome-evidence", sourceCommit: OSS,
    path: "apps/kernel/src/runtime/managed_release_update_evidence.rs",
    blob: "3a2391e350caf9b232b28aa2c5151066161fa2b3",
    classification: "signed_release_deployment_control",
    rationale: "Outcome admission binds update/environment/machine/kernel/digests and terminal phase after recovery; opened root-owned evidence is bounded. This controls deployment, not provider runtime.",
    anchors: [["release_activation", "settled_report"], ["release_activation", "unit_settled"], ["release_activation", "read_evidence"], ["release_activation", "prepare_archive"]],
  },
  {
    id: "release-update-owned-storage", sourceCommit: OSS,
    path: "deploy/managed-kernel/release-update-storage.py",
    blob: "3d11c0e2e5bc041680a696e0b2aa57c019a19c1b",
    classification: "signed_release_deployment_control",
    rationale: "Only an exact attempt ID under the root-owned staging root is reclaimed; private inode-bound journals outlive deletion, and mounts/special files are refused. Ordinary cwd/HOME paths are not filtered.",
    anchors: [["protected_path_filter", "boundary"], ["protected_path_filter", "check_tree"], ["cleanup_selector", "read_journal"], ["cleanup_selector", "cleanup_locked"], ["release_activation", "prepare"]],
  },
  {
    id: "bounded-release-archive-admission", sourceCommit: OSS,
    path: "deploy/managed-kernel/extract-release.py",
    blob: "ef26fdda3bab7c1458378d0e4403d0b5faf0bcec",
    classification: "signed_release_deployment_control",
    rationale: "The staging helper rejects escaping paths, sparse/special members and excess metadata/storage before signature verification; recovery space and attempt-owned scratch are deployment admission controls.",
    anchors: [["protected_path_filter", "path_name"], ["release_activation", "space_available"], ["release_activation", "validate"], ["release_activation", "extract_release"]],
  },
  {
    id: "account-pairing-session-authority", sourceCommit: OSS,
    path: "apps/kernel/src/runtime/cloud_api_client/pairing.rs",
    blob: "a1a44ee2d37d59027fef835863c692423ca1cb29",
    classification: "shared_cloud_account_authority",
    rationale: "Explicit account-wide pairing requires an existing nonblank Cloud session and uses the shared authenticated transport; the same rule applies to ordinary and managed profiles.",
    anchors: [["managed_only_branch", "request_account_pairing_token"]],
  },
  {
    id: "account-client-session-selection", sourceCommit: OSS,
    path: "apps/kernel/src/runtime/cloud_api_client/account_client_token.rs",
    blob: "262b0d06ecb0f55ca969691977d677a12dff89d5",
    classification: "shared_cloud_account_authority",
    rationale: "Account client token issuance selects the operator session by removing the machine credential from a local clone only. No topology selector or durable profile mutation is introduced.",
    anchors: [["managed_only_branch", "issue_cloud_account_client_runtime_token"]],
  },
  {
    id: "terminal-client-authority-scope", sourceCommit: OSS,
    path: "apps/kernel/src/runtime/cloud_api_client/terminal_client.rs",
    blob: "763356b76a9a63411c9ade9485db20ce8a6584a8",
    classification: "shared_cloud_terminal_capability",
    rationale: "Account clients use the linked session; machine-only clients use a machine-qualified subject and one canonical target without friendly aliases. Shared kernel/client behavior requires matching Cloud enforcement.",
    anchors: [["managed_only_branch", "issue_cloud_terminal_client_token"], ["client_projection", "machine_client_subject"]],
  },
  {
    id: "pairing-control-http-session-authority", sourceCommit: CLOUD,
    path: "apps/api/src/http/pairing-control-auth.ts",
    blob: "846a6ac72e7a79992d9accad28fe32f7dc0c3774",
    classification: "shared_cloud_account_authority",
    rationale: "Generic pairing administration requires Bearer Cloud session/account operate authority; browser cookie auth is not added to the generic path. No managed-only authorization branch exists.",
    anchors: [["managed_only_branch", "authorizePairingControl"]],
  },
  {
    id: "identity-bound-session-revocation", sourceCommit: CLOUD,
    path: "apps/api/src/relay/identity-revocation-repository.ts",
    blob: "e7a7aaf06abec809ae7369abd737d0c8dbeb9822",
    classification: "shared_cloud_identity_cleanup",
    rationale: "Client/machine revocation sweeps ACTIVE exact-account identity-bound sessions under provisioning-compatible identity locks; unrelated and browser-only sessions are preserved.",
    anchors: [["cleanup_selector", "revokeClientForAccount"], ["cleanup_selector", "revokeMachineForAccount"]],
  },
  {
    id: "logout-bound-identity-session-cleanup", sourceCommit: CLOUD,
    path: "apps/api/src/cloud-session-logout-repository.ts",
    blob: "6cfb5c6ab1f096b075008cd1b7e359784d301b32",
    classification: "shared_cloud_identity_cleanup",
    rationale: "Explicit logout binds requested client/machine to the authenticated session and takes identity locks before session writes. A receipt is persisted atomically for explicit revokes; matching retries only acknowledge committed cleanup. Plain logout stays session-only.",
    anchors: [["cleanup_selector", "logoutCloudSession"]],
  },
  {
    id: "completed-explicit-logout-acknowledgement", sourceCommit: CLOUD,
    path: "apps/api/src/cloud-session-logout-receipt.ts",
    blob: "51a0afc8aa21de7b23d9f06f763c12e7d61c760b",
    classification: "shared_cloud_identity_cleanup",
    rationale: "Only an exact request digest, revoked session, still-revoked bound identity and original tombstone ID acknowledge completed explicit logout. Relink or another revoke invalidates the old receipt; acknowledgement grants no new mutation.",
    anchors: [["cleanup_selector", "createLogoutReceipt"], ["cleanup_selector", "matchesCompletedLogout"], ["cleanup_selector", "matchesRevocation"]],
  },
  {
    id: "shared-runtime-credential-admission", sourceCommit: CLOUD,
    path: "apps/api/src/runtime/credential-repository.ts",
    blob: "83738b8beb5eff68807bbb2a83bf6e61bcd518e6",
    classification: "shared_cloud_machine_authority",
    rationale: "Cloud sessions retain account policy. Machine authentication locks the exact active account/machine before checking and touching its active credential. This credential-kind boundary is shared across ordinary and managed machines.",
    anchors: [["managed_only_branch", "requireRuntimeCredential"], ["managed_only_branch", "requireActiveRuntimeMachine"]],
  },
  {
    "id": "machine-credential-subject-and-target-authority",
    "sourceCommit": "8e9c24e4be0e87062343f60cda66f4c153539f91",
    "path": "apps/api/src/runtime/machine-scope-repository.ts",
    "blob": "9fa05a8ecb89da757ac230b183ec9cb9d126f244",
    "ranges": [
      [
        1,
        159
      ]
    ],
    "classification": "shared_cloud_machine_authority",
    "rationale": "Machine credentials admit only their active account/Machine, owned realm and exact target. Reserved slice subjects must carry that Machine hash and exact bootstrap/runtime/recovery shapes; target claims remain immutable. Account/session policy, transient-client collision refusal and bounded discovery are shared. Paired OSS recovery/alias behavior and live parity require their own validation.",
    "anchors": [
      [
        "managed_only_branch",
        "authorizeMachineRuntimeToken"
      ],
      [
        "managed_only_branch",
        "authorizeMachineKernelManagement"
      ],
      [
        "managed_only_branch",
        "claimMachineKernelOwnership"
      ],
      [
        "managed_only_branch",
        "claimKernelOwnership"
      ],
      [
        "client_projection",
        "readOnlyDiscovery"
      ],
      [
        "managed_only_branch",
        "machineOwnedSliceKernelToken"
      ],
      [
        "managed_only_branch",
        "machineOwnsSliceWorkerRef"
      ]
    ]
  },  {
    id: "shared-runtime-token-authority-wiring", sourceCommit: CLOUD,
    path: "apps/api/src/runtime/token-repository.ts",
    blob: "1b7dda73c1aa5e3fb8cbde6ab06bacae0b2fbd63",
    classification: "shared_cloud_machine_authority",
    rationale: "Event-generator and token issuance invoke the shared machine guard inside the credential transaction, then retain the account/session policy. Token rows bind the authenticated session or issuer machine; session membership does not widen machine target authority.",
    anchors: [["managed_only_branch", "authorizeEventGeneratorManagementForAccount"], ["managed_only_branch", "issueRuntimeTokenForAccount"]],
  },
  {
    id: "shared-kernel-presence-ownership", sourceCommit: CLOUD,
    path: "apps/api/src/relay/targets-repository.ts",
    blob: "a8fe7b6b603a5d000009c18bc68b4087ca539e9e",
    classification: "shared_cloud_machine_authority",
    rationale: "Presence requires the caller's exact machine and owned realm, claims immutable canonical kernel ownership before upsert, and cannot rewrite the winning owner. Target projection and heartbeat freshness remain shared behavior.",
    anchors: [["client_projection", "upsertKernelPresenceForAccount"]],
  },
  {
    id: "ordinary-provider-control-scrub-and-utility-launch", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/provider/managed_isolation.rs",
    blob: "37ae2123aaea0e9eaf82374a3ef45018a69cc017",
    ranges: [[12, 212], [1017, 1046], [1358, 1467]],
    classification: "shared_provider_launch_policy",
    rationale: "The ordinary/Path-1 branch returns the official provider program unchanged after common cwd preflight and control-environment scrub. Utilities reuse that launch branch. The separate isolation-selector branch and its namespace internals are outside these inspected ranges.",
    anchors: [["managed_only_branch", "managed_provider_isolation_required"], ["managed_env_selector", "managed_provider_control_env_remove"], ["managed_only_branch", "apply_managed_provider_isolation"], ["managed_only_branch", "expose_runtime_directory_in_managed_namespace"], ["managed_only_branch", "managed_isolated_utility_launch"], ["managed_only_branch", "managed_isolated_utility_command"], ["managed_env_selector", "command_from_provider_launch"]],
  },
  {
    id: "official-provider-adapter-wiring", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/provider/registry.rs",
    blob: "1c985a28ea01e6c1b25dd7d7b93ee606aa2c9cb6",
    ranges: [[1, 259]],
    classification: "shared_provider_launch_policy",
    rationale: "Claude, Codex and OpenCode use their ordinary launch planners, shared cwd preflight and workspace-write policy before the same topology-controlled launch helper. Test-only adapters are separate; the file does not select a Path-1 SDK or alternate runtime.",
    anchors: [["protected_path_filter", "preflight_provider_working_directory"], ["managed_only_branch", "connect"]],
  },
  {
    id: "exact-kernel-state-working-directory-protection", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/git_worktree_placement.rs",
    blob: "7715d4f909e7f33448d585071f934d2427c9313c",
    ranges: [[7, 400]],
    classification: "shared_exact_control_path_protection",
    rationale: "Common cwd and repository preflights canonicalize paths and protect exact kernel/control state, including five configured slice-control children. Slice root/development/siblings stay eligible. Only the exact kernel-owned workflow-instance subtree is re-exposed; provider-home protection requires the separate isolation selector.",
    anchors: [["protected_path_filter", "preflight_working_directory"], ["protected_path_filter", "preflight_managed_repository_root"], ["protected_path_filter", "ordinary_working_directory_protection"], ["protected_path_filter", "kernel_owned_workflow_runtime_instance_root"]],
  },
  {
    id: "ordinary-workspace-directory-discovery", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/runtime/workspace_search.rs",
    blob: "64d1d8783c846010fde8b23332208e098336e36c",
    ranges: [[1, 421]],
    classification: "shared_exact_control_path_protection",
    rationale: "Discovery, exact path completion and creation use the same common cwd preflight. HOME/cwd roots remain; an explicit server repository root is prepended. Creation is checked before and after mkdir, and unreadable directory entries are skipped by ordinary filesystem policy.",
    anchors: [["protected_path_filter", "search_workspace_directories"], ["protected_path_filter", "create_workspace_directory"], ["repository_home_state_path", "workspace_search_roots"]],
  },
  {
    id: "path1-bootstrap-topology-selection", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/mod.rs",
    blob: "b9dbeab8ebe8bf8e4bae1eb6b9fd8d5f6d545e2e",
    ranges: [[43, 94]],
    classification: "explicit_deployment_topology_selection",
    rationale: "Bootstrap accepts only explicit path1/shared_host topology. Path-1 inherited shared-host and broker environment lists are control inputs for scrub; this range does not introduce a provider runtime fork.",
    anchors: [["managed_only_branch", "managed_provider_topology"]],
  },
  {
    id: "verified-path1-kernel-launch-and-broker-handoff", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/supervisor.rs",
    blob: "8f691b7fb7d44e031c6d078aada14daff164f473",
    ranges: [[182, 292], [458, 525]],
    classification: "deployment_bootstrap_and_kernel_slice_capability",
    rationale: "Verified Path-1 launch selects ordinary process HOME/login PATH and no provider isolation roots; shared-host roots are a separate branch. Handoff removes inherited controls and passes only a private kernel broker FD or a required marker. Provider launch scrub removes those controls before provider execution.",
    anchors: [["managed_only_branch", "spawn_kernel_with_handoff"], ["kernel_slice_broker_control", "spawn_with_broker_lease"]],
  },
  {
    id: "verified-path1-worker-kernel-launch", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/worker.rs",
    blob: "662f944b8f6e981ab3e6d5d9b5afb655163df9b2",
    ranges: [[524, 580]],
    classification: "deployment_bootstrap_and_kernel_slice_capability",
    rationale: "Disposable worker launch requires Path 1, uses the verified kernel and ordinary process HOME/login PATH, carries the server repository root and leased home identity, and reuses supervisor broker handoff. No synthetic provider HOME is selected.",
    anchors: [["managed_only_branch", "spawn_kernel"]],
  },
  {
    id: "path1-home-service-source-policy", sourceCommit: OSS_RUNTIME,
    path: "deploy/managed-kernel/chariox-path1-managed-bootstrap.service",
    blob: "d3ea954bf322d2bfdf04ac831a157e7c82dc728b",
    classification: "path1_deployment_service_source",
    rationale: "The explicit Path-1 unit supervises the verified bootstrap under the ordinary user HOME with system bootstrap PATH. Source has no provider namespace/Protect*/Private*/NoNewPrivileges/ReadWritePaths restrictions. Effective units and drop-ins still require live inspection.",
    anchors: [],
  },
  {
    id: "path1-worker-service-source-policy", sourceCommit: OSS_RUNTIME,
    path: "deploy/managed-kernel/chariox-disposable-worker-bootstrap.service",
    blob: "13327059e4bf4eb938fb4a341b95a1da1e1a8b79",
    classification: "path1_deployment_service_source",
    rationale: "The explicit Path-1 worker unit supervises the ordinary kernel HOME and has no provider namespace/Protect*/Private*/NoNewPrivileges/ReadWritePaths restrictions. StateDirectory allocates service state; effective units and drop-ins remain unverified.",
    anchors: [],
  },
  {
    id: "managed-waiting-room-placement-projection", sourceCommit: OSS_RUNTIME,
    path: "packages/kernel-client/src/waiting-room-runtime-placement.ts",
    blob: "62c0b0329376ca6d3cc9cdb5fb5f755b3d07f311",
    classification: "deployment_control_client_projection",
    rationale: "Waiting-room managed options project server lifecycle and exact machine/kernel readiness/revision bindings. Once ready, the projection selects the ordinary canonical kernel; it supplies no provider prompt/session authority or worker execution fork.",
    anchors: [["client_projection", "waitingRoomLaunchMachineOptions"], ["client_projection", "waitingRoomLaunchKernelOptions"], ["client_projection", "waitingRoomLaunchPlacement"]],
  },
  {
    id: "shared-provider-utility-capture-budget", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/local/provider_requests/catalog/probe_capture.rs",
    blob: "95422e6143a2b954242aa38d64177f151a93c23d",
    classification: "shared_provider_utility_resource_policy",
    rationale: "Every provider utility uses one deadline, combined stdout/stderr memory cap and per-stream read budget with platform cleanup. This helper has no managed topology selector. Platform implementation and runtime evidence are distinct from this common-loop inspection.",
    anchors: [["cleanup_selector", "capture"], ["cleanup_selector", "drain"]],
  },
  {
    id: "shared-provider-status-probe-admission", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/local/provider_requests/catalog.rs",
    blob: "4727d4f3bcaf457a6f582a595e96d732a796788e",
    ranges: [[410, 445], [888, 910]],
    classification: "shared_provider_utility_resource_policy",
    rationale: "Status/version probes invoke the bounded common capture and translate timeout/output-limit failure into inconclusive auth state. Claude version is resolved from the official harness and uses the common utility launcher; no managed-only request path is added.",
    anchors: [["cleanup_selector", "bounded_status_output"], ["cleanup_selector", "bounded_output"], ["managed_only_branch", "claude_version"]],
  },
  {
    "id": "machine-scoped-slice-creation-and-placement",
    "sourceCommit": "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    "path": "apps/kernel/src/slice/store.rs",
    "blob": "9ecfde8a5be7a91b4b235b0fc25821d9b76f6bca",
    "ranges": [
      [
        90,
        218
      ],
      [
        987,
        1037
      ],
      [
        1352,
        1381
      ]
    ],
    "classification": "shared_slice_identity_namespace",
    "rationale": "LocalDocker creation assigns a per-creation Machine-qualified worker reference; explicit references and SshDocker defaults retain compatibility. Replay preserves references. Starting canonical workers must match their Kernel reference, and exact lookup excludes the shared parent Machine from worker aliases. Friendly execution references use the separate guarded resolver; this exact identity lookup does not itself admit execution.",
    "anchors": [
      [
        "client_projection",
        "create"
      ],
      [
        "client_projection",
        "restore_records"
      ],
      [
        "managed_only_branch",
        "claim_starting_worker_identity"
      ],
      [
        "client_projection",
        "resolve_by_worker_kernel_ref"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."
    ]
  },  {
    "id": "hosted-slice-owner-and-token-scope",
    "sourceCommit": "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    "path": "apps/kernel/src/runtime/slice_command_executor/lifecycle.rs",
    "blob": "35372b6ed7548da3b3fa5f62199fb724dae4d827",
    "ranges": [
      [
        1063,
        1194
      ]
    ],
    "classification": "shared_slice_owner_authority",
    "rationale": "Hosted bootstrap/runtime/recovery subjects use the persisted worker reference after matching owner Kernel, host Machine and authenticated profile Machine. Provider-visible slices receive no home Cloud profile. This scoped token inspection does not establish live recovery or activation; the corrected recovery policy is inspected separately.",
    "anchors": [
      [
        "managed_only_branch",
        "local_docker_slice_relay"
      ],
      [
        "client_projection",
        "local_docker_slice_relay_for_config"
      ],
      [
        "managed_only_branch",
        "validate_hosted_slice_identity"
      ],
      [
        "managed_only_branch",
        "hosted_cloud_slice_relay_token"
      ],
      [
        "managed_only_branch",
        "activate_hosted_slice_relay_token"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."
    ]
  },  {
    id: "managed-bootstrap-machine-profile-shape", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/cloud.rs",
    blob: "746bdf16a934fa34f30795d52e0f9e7f3896145e",
    ranges: [[48, 64]],
    classification: "machine_bound_deployment_enrollment",
    rationale: "The deny-unknown-fields bootstrap profile contains only Cloud metadata and a machine credential; it has no account CloudSession/operator-token field. This is enrollment authority, not a provider account export.",
    anchors: [["managed_only_branch", "ManagedCloudRelayProfile"]],
  },
  {
    id: "managed-bootstrap-persisted-profile-conversion", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/mod.rs",
    blob: "b9dbeab8ebe8bf8e4bae1eb6b9fd8d5f6d545e2e",
    ranges: [[1190, 1208]],
    classification: "machine_bound_deployment_enrollment",
    rationale: "Bootstrap conversion explicitly clears client identity, CloudSession token and all token expiries while preserving only the exchanged machine credential and enrollment metadata. It does not copy an operator Cloud profile.",
    anchors: [["managed_only_branch", "persisted_profile"]],
  },
  {
    id: "worker-bootstrap-machine-profile-binding", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_bootstrap/worker.rs",
    blob: "662f944b8f6e981ab3e6d5d9b5afb655163df9b2",
    ranges: [[209, 222], [313, 468]],
    classification: "machine_bound_deployment_enrollment",
    rationale: "Worker exchange denies unknown response fields, validates exact allocation/machine/kernel/account/user/realm and converts through the machine-only profile before persisting. Restart validates its receipt binding. This does not certify arbitrary later user-authored profiles or opaque user-secret values.",
    anchors: [["managed_only_branch", "ExchangeResponse"], ["managed_only_branch", "prepare"], ["managed_only_branch", "validate_exchange"], ["managed_only_branch", "validate_profile"]],
  },
  {
    id: "managed-machine-profile-config-persistence", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/config/persisted_daemon.rs",
    blob: "e06e927312d9737b4d2ab73ba6ecfa2d6ff5694e",
    ranges: [[234, 246]],
    classification: "machine_bound_deployment_enrollment",
    rationale: "Managed enrollment replaces the persisted Cloud profile and clears previous direct relay token/url; it uses the private common config writer. The converter supplies a machine-only profile. Loading existing user-authored config is a distinct boundary.",
    anchors: [["repository_home_state_path", "persist_managed_cloud_relay_profile"]],
  },
  {
    id: "selected-official-provider-account-export", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/account_profile.rs",
    blob: "a9c651d499151a4a4f6eaab5b1a796660c50efc8",
    ranges: [[2309, 2388]],
    classification: "selected_user_capability_export",
    rationale: "Selected provider profiles export only bounded official Codex/Claude/OpenCode auth files; Claude scope uses its matching profile/Keychain seam. The function does not export Chariox Cloud-session config. Credential payloads were not inspected.",
    anchors: [["managed_only_branch", "export_managed_context_materialization"]],
  },
  {
    id: "selected-context-package-component-wiring", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_context/outbound_service.rs",
    blob: "d86b1fa6c47f813d6577706bb57ca63f580f9081",
    ranges: [[747, 966]],
    classification: "selected_user_capability_export",
    rationale: "The bound plan selects development, provider accounts, Git credentials and kernel Extensions; Cloud profile is used only for source owner metadata. No DaemonConfig/CloudSession is serialized into the package. SourceKernel deliberately transfers an encrypted user vault; arbitrary user-secret semantics are outside this source-only inspection.",
    anchors: [["managed_only_branch", "prepare_managed_context_package"]],
  },
  {
    id: "kernel-context-closed-component-payload", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_context/kernel.rs",
    blob: "bf868e763f1667514311da01e251b6dd0c894b89",
    ranges: [[64, 76]],
    classification: "selected_user_capability_export",
    rationale: "Kernel context payload holds binding metadata, protocol compatibility, Extensions, dependencies and an encrypted vault component. It contains no daemon Cloud-profile field; opaque user-secret contents are not classified here.",
    anchors: [["managed_only_branch", "KernelContextPayload"]],
  },
  {
    id: "kernel-extension-export-component-selection", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_context/kernel/export.rs",
    blob: "e3e05515d3c02ed3d1af2bb8f39e52d90d9cdecd",
    ranges: [[116, 208]],
    classification: "selected_user_capability_export",
    rationale: "Export constructs the closed payload from portable user Extensions/dependencies and the explicitly supplied peer-bound vault. It captures protocol compatibility and component budgets, not daemon/operator Cloud config. Transferred user credentials remain intentional capability inputs.",
    anchors: [["managed_only_branch", "export_kernel_context"]],
  },
  {
    id: "kernel-context-source-registry-selection", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/managed_context/kernel/source_snapshot.rs",
    blob: "1fb52fa07cfdfe0362800519f7b1f26acfbf248c",
    ranges: [[31, 105]],
    classification: "selected_user_capability_export",
    rationale: "The source snapshot copies specific user Extension/environment/connector/credential registries into a bounded private snapshot. It does not capture the daemon config or Cloud profile; user-authored capability data can still contain explicit secrets.",
    anchors: [["managed_only_branch", "capture_once"]],
  },
  {
    id: "openship-publication-container-controls", sourceCommit: CLOUD,
    path: "deploy/openship/patches/0016-v0.6.6-chariox-integration.patch",
    embeddedPath: "packages/adapters/src/runtime/docker.ts",
    blob: "144750f45f2037e66bf6c8e30aecb90c957da61f",
    classification: "separate_publication_container_configuration",
    rationale: "Only strict-publication-v1 selects the OpenShip publication container's egress network, read-only credential binds and constrained HostConfig. Controls are consumed before tenant environment emission; this patch does not select the Path-1 provider host service.",
    anchors: [["protected_path_filter", "charioxReadOnlyBinds"], ["managed_only_branch", "consumeCharioxRuntimeControls"], ["managed_service_restriction", "strictPublicationHostConfig"], ["client_projection", "deploymentPortBindings"]],
  },
  {
    id: "openship-colocation-health-proof", sourceCommit: CLOUD,
    path: "deploy/openship/patches/0016-v0.6.6-chariox-integration.patch",
    embeddedPath: "apps/api/src/modules/health/chariox-colocation.ts",
    blob: "144750f45f2037e66bf6c8e30aecb90c957da61f",
    classification: "shared_cloud_colocation_authentication",
    rationale: "A configured instance identity and nonce-bound HMAC prove co-location to the server-side deployment caller. This is control-plane health proof, not provider execution or a kernel session authority.",
    anchors: [["managed_only_branch", "createCharioxColocationAttestation"]],
  },
  {
    "id": "per-creation-machine-scoped-slice-ref",
    "sourceCommit": "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    "path": "apps/kernel/src/slice/worker_identity.rs",
    "blob": "7a10a6f3c0becbc991c29ffc95a26e403e4d00e5",
    "ranges": [
      [
        1,
        77
      ]
    ],
    "classification": "shared_slice_identity_namespace",
    "rationale": "The shared LocalDocker factory hashes Machine ID separately from an owner/local-name/fresh 256-bit nonce tuple. The parser accepts exactly two lowercase 64-hex fields; Machine admission additionally checks the exact Machine hash. Hosted admission rejects legacy or foreign references before Cloud requests. Private explicit-reference compatibility is decided separately from this structural parser.",
    "anchors": [
      [
        "client_projection",
        "new_local_docker_worker_ref"
      ],
      [
        "client_projection",
        "worker_ref_with_nonce"
      ],
      [
        "client_projection",
        "qualified_worker_ref_parts"
      ],
      [
        "managed_only_branch",
        "machine_scoped_slice_worker_ref"
      ],
      [
        "managed_only_branch",
        "require_hosted_slice_worker_ref"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."
    ]
  },
  {
    "id": "hosted-slice-credential-subject-admission",
    "sourceCommit": "65d381d3184bd1e8e3630e6de081d9b62be0646f",
    "path": "apps/kernel/src/runtime/cloud_api_client.rs",
    "blob": "0dc754ceeb32dc2996b1cbbcd796db9e55cee368",
    "ranges": [
      [
        262,
        313
      ]
    ],
    "classification": "shared_slice_owner_authority",
    "rationale": "Both runtime and recovery issuance require an existing Machine credential and reject a legacy/foreign canonical subject before network I/O, then use the shared runtime token transport and owner-target shape. No account session or Machine credential is materialized in the slice.",
    "anchors": [
      [
        "managed_only_branch",
        "issue_cloud_slice_runtime_token"
      ],
      [
        "managed_only_branch",
        "issue_cloud_slice_recovery_token"
      ]
    ]
  },
  {
    "id": "slice-owner-token-install-and-refresh",
    "sourceCommit": "65d381d3184bd1e8e3630e6de081d9b62be0646f",
    "path": "apps/kernel/src/runtime/router/slice_relay_token_bridge.rs",
    "blob": "abc41c8799e68d1396e500b13ff7f9f0dea2b44f",
    "ranges": [
      [
        1,
        427
      ]
    ],
    "classification": "shared_slice_owner_authority",
    "rationale": "Worker identity uses canonical daemon_id and authenticated parent Machine. Install/refresh bind recorded slice, owner, sender public key, exact subject/target/action/thumbprint and expiry before persisted token installation; recovery is scoped to the owner connection. The peer boundary verifies signed sender authority; no alternate provider/session path is introduced.",
    "anchors": [
      [
        "managed_only_branch",
        "managed_slice_relay_identity"
      ],
      [
        "managed_only_branch",
        "install_managed_slice_relay_token"
      ],
      [
        "managed_only_branch",
        "refresh_managed_slice_relay_token"
      ],
      [
        "client_projection",
        "managed_slice_relay_identity_for_config"
      ],
      [
        "managed_only_branch",
        "validate_managed_slice_relay_token"
      ],
      [
        "managed_only_branch",
        "validate_managed_slice_recovery_token"
      ]
    ]
  },
  {
    "id": "signed-slice-kernel-caller-projection",
    "sourceCommit": "65d381d3184bd1e8e3630e6de081d9b62be0646f",
    "path": "apps/relay/src/server/connection/support.rs",
    "blob": "65c1a34200d0f8496bdcf8c2bea718bb63ac68d7",
    "ranges": [
      [
        1080,
        1113
      ]
    ],
    "classification": "shared_slice_owner_authority",
    "rationale": "Signed slice Kernel subjects remain Kernel callers for owner activation/refresh despite registering their parent Machine. Ordinary home kernels still project signed exact registrations to Machine for execution leases. Relay remains scoped transport and does not inspect encrypted runtime payloads.",
    "anchors": [
      [
        "client_projection",
        "peer_request_identity"
      ],
      [
        "client_projection",
        "registration_daemon_is_exact_kernel_or_temporary_peer"
      ]
    ]
  },
  {
    "id": "slice-environment-parent-machine-exclusion",
    "sourceCommit": "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    "path": "apps/kernel/src/slice/store/environment.rs",
    "blob": "a226240839737a5bcf868a83874081c18e087691",
    "ranges": [
      [
        124,
        129
      ],
      [
        145,
        198
      ]
    ],
    "classification": "shared_slice_identity_namespace",
    "rationale": "Room collision checks compare canonical/observed worker Kernel references and non-parent legacy Machine references, excluding the shared parent Machine. Lifecycle relaunch may reuse only its exact active typed operation guard from the same Store and slice, then checks duplicate-worker protection and Room membership. Friendly execution admission uses the separate guarded resolver.",
    "anchors": [
      [
        "managed_only_branch",
        "has_shared_worker"
      ],
      [
        "client_projection",
        "worker_refs"
      ],
      [
        "managed_only_branch",
        "require_environment_use"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."
    ]
  },
  {
    "id": "slice-provisioned-canonical-and-display-identity",
    "sourceCommit": "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    "path": "apps/kernel/src/slice/local_docker.rs",
    "blob": "c95558b142a57383128c6a6038be038a5b5eea37",
    "ranges": [
      [364, 367],
      [
        1066,
        1146
      ],
      [
        1193,
        1195
      ]
    ],
    "classification": "shared_slice_identity_projection",
    "rationale": "Provisioning projects canonical worker reference separately from display alias. Hosted workers use their parent Machine and private workers retain synthetic identities. Both ordinary and broker placement receive the common selected image. The extension adapter prepares its image before broker dispatch and removes the raw private Dockerfile path; guarded execution admission handles friendly aliases separately.",
    "anchors": [
      [
        "client_projection",
        "configure_local_docker_slice_command"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."
    ]
  },
  {
    "id": "inner-slice-kernel-bootstrap-identity",
    "sourceCommit": "65d381d3184bd1e8e3630e6de081d9b62be0646f",
    "path": "apps/kernel/slice-linux-docker/docker/start-runtime.sh",
    "blob": "195ce5230578b6ed93068918d0d7f9ce9227c73e",
    "ranges": [
      [
        22,
        28
      ],
      [
        132,
        169
      ]
    ],
    "classification": "shared_inner_slice_bootstrap_projection",
    "rationale": "The inner Docker kernel receives separate canonical ID and friendly alias plus parent/owner binding. Its provider isolation controls belong to the explicit Docker slice, used by ordinary and managed hosts, and are not selected for Path-1 host provider children. This does not clear host broker capability differences.",
    "anchors": [
      [
        "client_projection",
        "start_slice_kernel"
      ]
    ]
  },
  {
    "id": "slice-provisioner-container-identity-forwarding",
    "sourceCommit": "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "7a6a020cf479699ca9c56a7fbf693e75617efa34",
    "ranges": [
      [
        109,
        112
      ],
      [
        1113,
        1195
      ]
    ],
    "classification": "shared_inner_slice_bootstrap_projection",
    "rationale": "The common provisioner forwards canonical daemon ID separately from display alias and Machine into docker exec, including explicit Room binding. Hosted callers omit the home Cloud profile. This identity projection is shared; the image-build/cache path is inspected in its separate rule.",
    "anchors": [
      [
        "client_projection",
        "exec_slice_with_timeout"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."
    ]
  },
  {
    id: "path1-broker-prepared-image-admission",
    sourceCommit: "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    path: "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    blob: "6d52a86d75f32c1e1c6d0d55775aa7cbd45a6c93",
    ranges: [[115, 164], [335, 436], [438, 551]],
    classification: "shared_slice_image_operand_admission",
    rationale: "The broker consumes the already prepared extension image through existing provisioner policy; it continues rejecting raw caller Dockerfile paths in broker requests. Read-only image inspection, snapshot helper image argument17, and provisioner image/base fields use typed Docker references. Container/volume ownership, matching read-only helper mounts and image commit/removal ownership remain separate checks. The signed helper and sender supply the ordinary extension capability before this boundary.",
    anchors: [["client_projection", "ALLOWED_ENVIRONMENT"], ["managed_only_branch", "validateDocker"], ["managed_only_branch", "validateProvisioner"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    "id": "canonical-slice-cloud-recovery-shape",
    "sourceCommit": "8e9c24e4be0e87062343f60cda66f4c153539f91",
    "path": "apps/api/src/runtime/token-policy.ts",
    "blob": "deb05fd4441c34fc2ef892e187839f9e67e623f2",
    "ranges": [
      [
        132,
        155
      ]
    ],
    "classification": "shared_cloud_machine_authority",
    "rationale": "Canonical slice refs have exact Machine/creation hash shape. Recovery requires a key thumbprint, one owner target and exact register/heartbeat/peer-request actions; repository admission separately checks Machine hash and owned parent target. The restriction is shared token authority, not a Path-1 runtime selector.",
    "anchors": [
      [
        "managed_only_branch",
        "isCanonicalSliceWorkerRef"
      ],
      [
        "managed_only_branch",
        "managedSliceRecoveryTokenShape"
      ]
    ]
  },
  {
    id: "shared-exact-slice-binding-recovery",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/app/remote_agent_binding.rs",
    blob: "f6da84020a45993b4da15b92498bb56a0702bcae",
    ranges: [[45, 119], [419, 439], [651, 690], [1002, 1219], [1471, 1550]],
    classification: "shared_slice_binding_recovery",
    rationale: "Binding preparation admits the recorded slice before key/lease I/O and retains its Room guard through the result and binding commit. For recorded slice bindings, both refresh paths select the exact worker through shared recovery policy; lifecycle relaunch can use its own typed operation guard. Generic Machine selection excludes reserved and recorded custom slice workers. Exact slice selection checks the persisted worker identity against relay metadata. The caller retains shared Room admission while spawn_worker_agent selects and binds the worker.",
    anchors: [["managed_only_branch", "execute_remote_agent_binding_refresh"], ["client_projection", "spawn_worker_agent"], ["managed_only_branch", "prepare_remote_agent_binding_refresh"], ["managed_only_branch", "refresh_remote_agent_binding"], ["managed_only_branch", "refresh_remote_agent_binding_to_worker_kernel_with_operation"], ["client_projection", "select_remote_kernel_by_ref_with_config"]],
    openFindings: ["Fresh ordinary/Path-1 recovery and Room isolation comparison and independent semantic disposition remain pending."],
  },
  {
    id: "room-guarded-slice-execution-admission",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/state/room_environment_placement.rs",
    blob: "ec0c70e35d33a7759d328b6763373567b6bf4885",
    ranges: [[12, 49]],
    classification: "shared_room_slice_admission",
    rationale: "Room execution resolves explicit slice refs or the shared guarded execution resolver, including friendly aliases. It rejects simultaneous slice/kernel refs, preserves target order and holds one physical Environment guard per unique slice through caller execution. Unknown reserved aliases and ambiguous references fail in the resolver.",
    anchors: [["managed_only_branch", "guard_slice_execution"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "agent-spawn-guarded-slice-placement",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/session_actor/store/agent.rs",
    blob: "20de20fc035947b669caf612eede1eb52b8123b5",
    ranges: [[175, 203], [262, 327]],
    classification: "shared_room_slice_admission",
    rationale: "Agent spawn holds shared execution admission, uses its resolved slice identity for worktree-scope validation and canonical worker selection, then attaches the created agent to that same slice. Friendly aliases now enter this path through the corrected guarded resolver. These scoped caller ranges do not establish end-to-end provider behavior.",
    anchors: [["client_projection", "spawn_agent"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "stale-prompt-guarded-binding-refresh",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/state/remote_prompt_worker_submission_runtime.rs",
    blob: "f1c35560cced01aa63bbf53cd0d7435eb5c08ac8",
    ranges: [[580, 611]],
    classification: "shared_slice_binding_recovery",
    rationale: "Automatic stale-prompt recovery invokes the common binding refresh and updates dispatch only from the resulting remote binding. The inspected callee retains exact slice Kernel/Machine identity and Room admission instead of generic parent-Machine failover. The caller itself does not select a worker or introduce a managed-only prompt path.",
    anchors: [["managed_only_branch", "refresh_remote_prompt_binding"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "guarded-friendly-slice-reference-resolution",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/slice/store/execution_reference.rs",
    blob: "cfd0f380b87f21af7cdbf1a8bb40614c2acd2c6d",
    ranges: [[1, 41]],
    classification: "shared_room_slice_admission",
    rationale: "Guarded execution resolves persisted reference, observed Kernel, non-parent observed Machine and the local friendly slice name under one Store lock. It rejects multiple matches and unresolved reserved slice references. Exact authenticated peer/token lookup remains separate. Callers must retain the physical Environment guard; this resolver alone does not authorize execution.",
    anchors: [["managed_only_branch", "resolve_execution_worker_kernel_ref"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "recorded-slice-recovery-policy",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/app/remote_agent_binding/slice_recovery.rs",
    blob: "3148f1449c4c858c0c05106978a3f419548af4c8",
    ranges: [[18, 125], [136, 180], [192, 207]],
    classification: "shared_slice_binding_recovery",
    rationale: "Recovery requires a unique recorded slice, checks observed Kernel/Machine before and after Room guard admission, and admits only the same relay worker with provider eligibility. Canonical owner-Machine references retain their persisted identity; private custom workers retain recorded observed IDs. The guard survives until binding commit. Generic Machine failover excludes reserved or locally recorded slice identities. No managed-only provider path is introduced.",
    anchors: [["managed_only_branch", "admit"], ["managed_only_branch", "select"], ["managed_only_branch", "require_worker"], ["client_projection", "recorded_slice_worker_id"], ["managed_only_branch", "require_selected_slice_worker"], ["client_projection", "recorded_worker_id"], ["managed_only_branch", "ordinary_machine_workers"]],
    openFindings: ["Fresh ordinary/Path-1 recovery and Room isolation comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-local-worker-context",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/slice/worker_context.rs",
    blob: "9d8313104a54e09c4713ebdd76cb1b4aead5c8b2",
    ranges: [[5, 50]],
    classification: "shared_slice_worker_context",
    rationale: "Classifies the local execution context using canonical daemon identity and actual parent Machine with an explicit local slice ID. Private synthetic slice:<localID> Machine compatibility remains. Prompt uses launch environment; MCP and physical tool callers use config. A Room binding is optional, so headless/provider context remains classified. Advertised aliases are absent from this decision.",
    anchors: [["client_projection", "slice_worker_id"], ["client_projection", "current_slice_worker_id"], ["client_projection", "slice_worker_id_for_config"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-recorded-attachment-identity",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/slice/worker_context.rs",
    blob: "9d8313104a54e09c4713ebdd76cb1b4aead5c8b2",
    ranges: [[52, 93], [95, 165]],
    classification: "shared_slice_attachment_identity",
    rationale: "Missing, unique and ambiguous matches remain distinct. Existing stopped private attachments with neither observed Kernel nor Machine may survive Missing. This fallback never creates an attachment, never accepts ambiguity, and excludes owner-matching canonical identities and recorded hosted relay placements. Restoration resolves a unique exact Kernel/Machine pair over the full recorded slice list. Owner-matching canonical refs reject observed Kernel disagreement and foreign parent Machine. Private explicit SSH aliases that happen to look qualified require both persisted observed identities; Cloud login alone does not turn a private/self-hosted relay into hosted placement. Actual private:false endpoint equal to the Cloud profile relay URL rejects a foreign qualified alias. Unknown endpoints require exact observations. No relay aliases or independent OR matching grants ownership.",
    anchors: [["client_projection", "RecordedSliceWorker"], ["managed_only_branch", "recorded_slice_for_worker"], ["managed_only_branch", "retained_slice_attachment_matches"], ["managed_only_branch", "recorded_relay_is_hosted"], ["managed_only_branch", "recorded_slice_worker"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "room-worker-binding-scope",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/config/room_environment.rs",
    blob: "d70d46dad413dfec2fe1c6a9713ebbc9a5dcd634",
    ranges: [[34, 64]],
    classification: "shared_room_slice_authority",
    rationale: "Upgrade-owned validator admits an actual parent Machine only when canonical daemon identity matches its Machine hash; synthetic private Machine remains accepted. Home Kernel, canonical public key, Room and local slice fields remain required. permits still checks all four provisioner-owned values exactly. This audit does not broaden receiver permissions.",
    anchors: [["managed_only_branch", "validate"], ["managed_only_branch", "permits"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-provider-prompt-context",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/prompt_assembly.rs",
    blob: "e88d3ba96fd58f6053e6eefe5fbbdcb8967d2556",
    ranges: [[827, 842], [988, 990]],
    classification: "shared_slice_provider_context",
    rationale: "The normal provider turn assembly adds runtime/slice through the shared local classifier. No separate managed prompt or provider execution path is added. Delegates environment-based classification to current_slice_worker_id. Friendly alias text on an ordinary Machine is insufficient.",
    anchors: [["client_projection", "assemble_provider_turn"], ["managed_only_branch", "current_kernel_is_slice"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-isolated-mcp-materialization",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/app/remote_lease/mcp_availability.rs",
    blob: "7ad0a913b508beb7feba343a78750090bfc2b5f2",
    ranges: [[231, 280]],
    classification: "shared_slice_provider_context",
    rationale: "Only a classified slice with a configured capability isolation root materializes required MCP definitions into its isolated registry. Ordinary workers retain operator-installed capabilities. Existing backing-session and exact lease home Kernel/Room/Agent context checks remain. Classification does not replace lease authentication.",
    anchors: [["managed_only_branch", "ensure_isolated_required_mcp_definitions"], ["managed_only_branch", "validate_mcp_check_context"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-runtime-tool-advertisement-and-routing",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/state/tool_dispatch.rs",
    blob: "717826c4efeb2aaeec0cb064f0b54c90c9378bdb",
    ranges: [[422, 446], [457, 517]],
    classification: "shared_slice_computer_routing",
    rationale: "The scoped authenticated dispatch branch attempts the home Room browser route, retains leased Room browser restrictions, then invokes shared slice physical dispatch. Tool advertisement and execution use the same config classifier, with existing session Environment or granted home manifest for Room tools. Friendly aliases on ordinary machines do not expose local screen tools. Authentication before this branch is outside these scoped ranges.",
    anchors: [["client_projection", "slice_tool_specs_for_provider_runs"], ["client_projection", "room_browser_slice_for_tool"], ["client_projection", "slice_kernel_id"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-physical-tool-dispatch",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/state/tool_dispatch/slice.rs",
    blob: "a2b9e4f8877de4fb0792e49b35d69f94b8045238",
    ranges: [[22, 82]],
    classification: "shared_slice_computer_routing",
    rationale: "The scoped dispatch entry selects existing Room Environment placement or the classified local slice, requires an agent-bound provider run, and preserves controller routing before the physical screen implementation. Hosted parent-Machine classification no longer falsely omits tools. Later tool implementations and image payload handling are outside this scoped admission inspection; no home-screen fallback is added.",
    anchors: [["client_projection", "dispatch_slice_runtime_tool_call"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "restored-exact-slice-attachments",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/app/durable_runtime_state.rs",
    blob: "9f6bbc4c26a8e3674de54f5992c83f451e9ccb25",
    ranges: [[674, 720]],
    classification: "shared_slice_attachment_identity",
    rationale: "Durable restoration calls the unique exact full-list matcher before adding a missing attachment. It preserves existing event append behavior and does not use the stopped retention fallback for new attachments.",
    anchors: [["client_projection", "reconcile_restored_slice_agent_attachments"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "lifecycle-exact-slice-attachments",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/state/slice_runtime_state.rs",
    blob: "74f217ae9c1153a581b163daecaf95946a9f9cab",
    ranges: [[96, 182]],
    classification: "shared_slice_attachment_identity",
    rationale: "Lifecycle reconciliation uses the same full-list exact matcher for new attachments and the separate existing-attachment retention policy. Canonical Agent/Room existence and session cleanup checks remain. Conflicting Machine, reused synthetic ID and duplicate exact workers cannot add or preserve active attachments.",
    anchors: [["client_projection", "reconcile_slice_agent_attachments"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },
  {
    id: "slice-lifecycle-attachment-reconciliation",
    sourceCommit: "9cad5ea652c8ed7286a51521f03eec13a1a6f29c",
    path: "apps/kernel/src/runtime/slice_command_executor/lifecycle.rs",
    blob: "35372b6ed7548da3b3fa5f62199fb724dae4d827",
    ranges: [[119, 135], [449, 465], [566, 584]],
    classification: "shared_slice_attachment_identity",
    rationale: "Save, backup restore and start reconcile attachments before lifecycle manifest or materialization work while retaining their existing operation guards. The shared exact matcher and stopped-private retention policy determine attachment identity. These entry ranges do not establish complete lifecycle success or live worker availability.",
    anchors: [["client_projection", "execute_save_slice_state_request"], ["client_projection", "execute_restore_slice_backup_request"], ["client_projection", "execute_start_slice_request_with_relaunch_manifests"]],
    openFindings: ["Fresh ordinary/Path-1 runtime comparison and independent semantic disposition remain pending."],
  },

  {
    id: "shared-extension-image-cache-identity",
    sourceCommit: "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    path: "apps/kernel/src/slice/local_docker/image.rs",
    blob: "43da416ec1a8fecdd3b1afebade5457cfa525e38",
    ranges: [[1, 46]],
    classification: "shared_slice_image_identity",
    rationale: "Extensions using the default base or its exact Docker Hub spellings select a creation-specific image from length-framed owner Kernel/Machine, local slice ID and persisted nonce-bearing worker reference. Explicit custom images and saved-state images retain selection. Ordinary and Path-1 paths share this policy; registry domain case is preserved.",
    anchors: [["client_projection", "selected_image"], ["client_projection", "is_default_runtime_base"], ["client_projection", "choose_image"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    id: "path1-extension-build-sender",
    sourceCommit: "3646a8865d508e1f08c85ae683e83297e35d7252",
    path: "apps/kernel/src/slice/local_docker/extension_build.rs",
    blob: "a7f2c8b2536b75f07d0a739d71d6fbdd578eb5f1",
    ranges: [[1, 305]],
    classification: "path1_extension_build_adapter",
    rationale: "Only broker-backed provision with an extension and no saved archive invokes the signed helper. Cache admission precedes opening user context; live caller descriptors pin the selected Dockerfile target and context. The isolated privileged interpreter receives raw environment bytes encoded over stdin and restores settings only after privilege drop. Native-derived bounds replace small managed limits. The prepared image returns to existing broker placement with build policy never. Recovery and saved-state paths retain their existing behavior.",
    anchors: [["managed_only_branch", "prepare"], ["managed_only_branch", "open_dockerfile"], ["managed_only_branch", "capture_client_environment"], ["managed_only_branch", "helper_command"], ["managed_only_branch", "run_helper"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    id: "path1-pinned-user-extension-helper",
    sourceCommit: "6c2630e550335d15e7993d948331afa75454674d",
    path: "apps/kernel/slice-linux-docker/managed-extension-build.py",
    blob: "bdec9e3c188b3e3270aab62a619fd46cdd45110d",
    ranges: [[1, 517]],
    classification: "path1_extension_build_adapter",
    rationale: "The installed helper binds its signed context, full-sudo caller, source descriptors, cwd and protected rootless socket. Read-only context projection preserves native Docker ignore/COPY semantics. The child drops UID/capabilities before caller environment restoration and preserves the caller's existing sudo/setuid authority; endpoint, builder and shell controls remain helper-owned. Socket access adds only the pinned daemon primary or uniquely allocated subordinate GID temporarily. Caller/monitor-bound PID namespaces settle descendants; output retention is bounded without a build deadline. Cleanup removes only owned empty leases. This adapts placement for an already root-capable user and does not establish deployed parity.",
    anchors: [["managed_only_branch", "normalized_image_reference"], ["managed_only_branch", "validate_request"], ["managed_only_branch", "path1_user"], ["managed_only_branch", "pin_invoker"], ["managed_only_branch", "open_source"], ["managed_only_branch", "pin_cwd"], ["managed_only_branch", "daemon_socket_group"], ["managed_only_branch", "pin_socket"], ["managed_only_branch", "signed_digest"], ["managed_only_branch", "project_context"], ["managed_only_branch", "drop_user"], ["managed_only_branch", "namespace_build"], ["managed_only_branch", "main"], ["cleanup_selector", "scratch_lease"], ["cleanup_selector", "capture_build"], ["cleanup_selector", "run_namespace"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    id: "shared-provisioner-extension-cache-and-build",
    sourceCommit: "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    path: "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    blob: "7a6a020cf479699ca9c56a7fbf693e75617efa34",
    ranges: [[9, 16], [659, 804], [1610, 1637]],
    classification: "shared_slice_image_build_policy",
    rationale: "The helper reuses the signed common build_image path. Cache/saved-image decisions run before check-only exit42 and before opening the extension context. A helper-owned socket selects the local default builder; ordinary placement retains its Docker command selection. Extension builds retain the original build arguments and accept the pinned context projection. The internal build-image action invokes existing admission and image build without provisioning a container.",
    anchors: [["client_projection", "docker"], ["client_projection", "docker_target_arch"], ["client_projection", "docker_build"], ["client_projection", "build_standard_runtime_image"], ["client_projection", "ensure_runtime_base_image"], ["client_projection", "build_image"], ["client_projection", "main"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    id: "typed-docker-image-reference-grammar",
    sourceCommit: "d76dea8c6745aac5dc53eaa37284dfa4fe394bec",
    path: "apps/kernel/slice-linux-docker/docker-image-reference.mjs",
    blob: "8a4bd2fc28e47a444578a8ebf94771b2b11c3108",
    ranges: [[1, 24]],
    classification: "shared_slice_image_operand_admission",
    rationale: "Image operands use the adjacent pinned distribution-reference grammar and exact Docker Hub normalization, preserving domain case. Registry/port/tag/digest operands remain distinct from container and volume ownership validation. The grammar is packaged inside the signed build context and is consumed by Python and JavaScript; Rust shares the tested default-base equivalence.",
    anchors: [["client_projection", "normalizedImageReference"], ["client_projection", "isDockerImageReference"], ["client_projection", "isDefaultRuntimeBase"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },

  {
    id: "typed-docker-image-reference-policy-data",
    sourceCommit: "3646a8865d508e1f08c85ae683e83297e35d7252",
    path: "apps/kernel/slice-linux-docker/docker-image-reference.json",
    blob: "455e3bcfbdafe168026ae063627d189e68a31807",
    ranges: [[1, 6]],
    classification: "shared_slice_image_operand_admission",
    rationale: "The exact JSON sidecar binds the image-reference expression, repository-path bound and default runtime base consumed by the Python and JavaScript adapters. Registry/tag/digest grammar applies only to typed image operands, with separate resource mutation ownership. This data inspection does not establish registry availability or live image compatibility.",
    anchors: [["managed_only_branch", "reference"], ["managed_only_branch", "maximumRepositoryPath"], ["managed_only_branch", "defaultBase"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution and independent semantic disposition remain pending."],
  },
]);

function rulesFor(file, embeddedPath, lineNumber = null) {
  return SOURCE_AUDIT_RULES.filter((rule) => rule.path === file.path
    && (rule.embeddedPath ?? null) === (embeddedPath ?? null)
    && (lineNumber === null || !rule.ranges || rule.ranges.some(([start, end]) => lineNumber >= start && lineNumber <= end)));
}

export function sourceRuleCandidates(file, lines, embeddedPath = null) {
  const candidates = [];
  for (const rule of rulesFor(file, embeddedPath)) {
    for (const [category, symbol] of rule.anchors) {
      const declaration = new RegExp("\\b(?:fn|function|def)\\s+" + symbol + "(?:<[^\\n]*>)?\\s*\\(|\\b(?:struct|enum|interface|type|const)\\s+" + symbol + "\\b|(?:^|\\s)" + symbol + "\\s*\\(\\s*\\)\\s*\\{");
      // Explicitly inspected JSON policy keys are declarations too. This is
      // limited to a rule's named keys and ranges; it grants no semantic approval.
      const jsonProperty = file.path.endsWith(".json")
        ? new RegExp("^\\s*" + JSON.stringify(symbol) + "\\s*:") : null;
      for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
        const match = declaration.exec(lines[lineIndex]) ?? jsonProperty?.exec(lines[lineIndex]);
        if (match && (!rule.ranges || rule.ranges.some(([start, end]) => lineIndex + 1 >= start && lineIndex + 1 <= end))) candidates.push({
          category, symbol, selector: symbol, lineIndex,
          column: lines[lineIndex].indexOf(symbol) + 1,
          affectedBehavior: rule.rationale, ruleId: rule.id,
          applicableMpIds: category === "automatic_shutdown_selector" ? ["MP-09", "MP-11"]
            : category === "release_activation" ? ["MP-07", "MP-11"]
            : category === "protected_path_filter" ? ["MP-02", "MP-03", "MP-11"]
              : category === "cleanup_selector" ? ["MP-08", "MP-10", "MP-11"] : ["MP-08", "MP-11"],
        });
      }
    }
  }
  return candidates;
}

export function sourceClassification(file, embeddedPath = null, lineNumber = null) {
  const rule = rulesFor(file, embeddedPath, lineNumber)[0];
  if (!rule) return null;
  const inspected = rule.blob === file.blob;
  return {
    ruleId: rule.id, status: inspected ? "source_inspected" : "source_drift",
    classification: inspected ? rule.classification : null,
    auditSourceCommit: rule.sourceCommit, auditedBlob: rule.blob,
    rationale: inspected ? rule.rationale : "The inspected source blob changed; re-inspection is required.",
    auditedRanges: rule.ranges ?? null,
    openFindings: inspected ? (rule.openFindings ?? []) : [],
    authoritative: false, independentDisposition: "pending",
  };
}

export function groupSourceClassifications(entries) {
  const groups = new Map();
  for (const entry of entries) {
    const observation = entry.sourceClassification;
    if (!observation) continue;
    const key = observation.ruleId + ":" + entry.blob;
    if (!groups.has(key)) groups.set(key, {
      ...observation, path: entry.path, blob: entry.blob,
      embeddedPath: entry.patchSource?.path ?? null, candidateIds: [],
    });
    groups.get(key).candidateIds.push(entry.candidateId);
  }
  return [...groups.values()].sort((left, right) => left.ruleId.localeCompare(right.ruleId));
}



export function sourceAuditGaps(files, locatedAnchors) {
  const byPath = new Map(files.map((file) => [file.path, file]));
  const cloud = files.some((file) => file.path.startsWith("apps/api/"));
  const oss = files.some((file) => file.path.startsWith("apps/kernel/"));
  const gaps = [];
  for (const rule of SOURCE_AUDIT_RULES) {
    const file = byPath.get(rule.path);
    const applies = (rule.sourceCommit === CLOUD || rule.sourceCommit === CLOUD_CURRENT) ? cloud : oss;
    if (!file) {
      if (applies) gaps.push({ kind: "expected_source_missing", ruleId: rule.id, path: rule.path,
        auditedBlob: rule.blob, auditSourceCommit: rule.sourceCommit, authoritative: false });
      continue;
    }
    for (const [category, symbol] of rule.anchors) {
      if (locatedAnchors.has(rule.id + ":" + symbol)) continue;
      gaps.push({ kind: "expected_declaration_missing", ruleId: rule.id, path: rule.path,
        embeddedPath: rule.embeddedPath ?? null, category, symbol,
        blob: file.blob, auditedBlob: rule.blob, auditSourceCommit: rule.sourceCommit,
        authoritative: false });
    }
  }
  return gaps;
}
