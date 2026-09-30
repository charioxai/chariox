// Scoped source audit rules are provisional observations, never independent
// semantic approvals. Each rule binds an inspected blob; changed blobs retain
// candidates but lose the classification until re-inspected.
const OSS = "b375eb6eec1a6700c329830fd3bfba581eb818dc";
const OSS_RUNTIME = "0e250d53977a49a73c3c3ee0f9251d9f9febe712";
const CLOUD = "a8f5ee1bc80f13f508cf950752d779399df1aa55";
export const SOURCE_AUDIT_RULES = Object.freeze([
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
    id: "machine-credential-subject-and-target-authority", sourceCommit: CLOUD,
    openFindings: ["Historical MP-08/MP-11 at OSS 0e / Cloud a8: local-name slice refs collide under immutable kernel ownership. Cloud 978 publishes new admission; exact combined OSS mapping/relay/compatibility evidence remains pending."],
    path: "apps/api/src/runtime/machine-scope-repository.ts",
    blob: "80e8a354502369d0f398c2758be96d10c0717206",
    classification: "shared_cloud_machine_authority",
    rationale: "Machine credentials authorize only their own active account/machine, realm and canonical targets. Kernel first-claim ownership is immutable; terminal clients use a machine-qualified namespace without paired-client collisions. Read-only discovery and inner-slice token shapes are separately bounded; no Path-1 provider launch selector is introduced.",
    anchors: [["managed_only_branch", "authorizeMachineRuntimeToken"], ["managed_only_branch", "authorizeMachineKernelManagement"], ["managed_only_branch", "claimMachineKernelOwnership"], ["managed_only_branch", "claimKernelOwnership"], ["client_projection", "readOnlyDiscovery"], ["managed_only_branch", "machineOwnedSliceKernelToken"]],
  },
  {
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
    id: "local-slice-name-canonical-ref-collision", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/slice/store.rs",
    blob: "363eff5a0536779ecf5e8025b6c75ead4fd84f88",
    ranges: [[89, 121]],
    classification: "shared_runtime_defect",
    rationale: "Local slice names are unique only inside one SliceStore, but the default worker ref slice:<name> is later used as an account/realm-wide canonical Cloud identity. Equal local names on different ordinary or managed machines collide.",
    openFindings: ["Historical MP-08/MP-11 at OSS 0e / Cloud a8: same-name slices contend for immutable Cloud ownership. Cloud 978 admission is published; combined canonical mapping/migration evidence remains pending."],
    anchors: [["client_projection", "create"]],
  },
  {
    id: "hosted-slice-ref-identity-use", sourceCommit: OSS_RUNTIME,
    path: "apps/kernel/src/runtime/slice_command_executor/lifecycle.rs",
    blob: "bf303266acbc2c12d9fcdedf6eaf68c3c9d97bd9",
    ranges: [[1118, 1169]],
    classification: "shared_runtime_defect",
    rationale: "Hosted bootstrap/runtime/recovery token requests reuse worker_kernel_ref as a KERNEL subject. At this blob local-name defaults can collide across machines; credential confinement must retain the owned parent while the canonical mapping is corrected.",
    openFindings: ["Historical MP-08/MP-11 at OSS 0e / Cloud a8: slice:<local name> is not machine-qualified. Cloud 978 admission is published; combined OSS ref/relay compatibility and exact-source tests remain pending."],
    anchors: [["managed_only_branch", "hosted_cloud_slice_relay_token"], ["managed_only_branch", "activate_hosted_slice_relay_token"]],
  },
  {
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
      const declaration = new RegExp("\\b(?:fn|function|def)\\s+" + symbol + "(?:<[^\\n]*>)?\\s*\\(|\\b(?:struct|enum|interface|type)\\s+" + symbol + "\\b");
      for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
        const match = declaration.exec(lines[lineIndex]);
        if (match && (!rule.ranges || rule.ranges.some(([start, end]) => lineIndex + 1 >= start && lineIndex + 1 <= end))) candidates.push({
          category, symbol, selector: symbol, lineIndex,
          column: lines[lineIndex].indexOf(symbol) + 1,
          affectedBehavior: rule.rationale, ruleId: rule.id,
          applicableMpIds: category === "release_activation" ? ["MP-07", "MP-11"]
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
    const applies = rule.sourceCommit === CLOUD ? cloud : oss;
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
