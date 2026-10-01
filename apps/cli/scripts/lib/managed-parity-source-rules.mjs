// Scoped source audit rules are provisional observations, never independent
// semantic approvals. Each rule binds an inspected blob; changed blobs retain
// candidates but lose the classification until re-inspected.
const OSS = "686ec57d5e46cdd46e723155b7eb89f6f25202a2";
const OSS_RUNTIME = "686ec57d5e46cdd46e723155b7eb89f6f25202a2";
const CLOUD = "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2";
const CLOUD_CURRENT = "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2";
const CLOUD_PARITY3 = "d0638173d10e1fa6040741d72da455d9847b4322";

const CLOUD_IMPLD = "8fac1a2c8bc5c208ce4d71a43ff978b8b070a451";
export const SOURCE_AUDIT_RULES = Object.freeze([
  {
    "id": "broker-runtime-output-budget",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        45,
        45
      ],
      [
        1590,
        1672
      ],
      [
        1685,
        1685
      ]
    ],
    "classification": "shared_streaming_output_with_private_broker_logs",
    "rationale": "MP-08/MP-11: runtime commands use the common process-group owner, stream complete diagnostics to private logs and retain bounded response tails. Actual broker output above4MiB succeeds and a second request runs. Log files and directories are0600/0700; configured share roots own their log paths. No client protocol shape is changed.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "MAX_OUTPUT_BYTES"
      ],
      [
        "kernel_slice_broker_control",
        "execute"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "placement-selected-slice-sandbox-policy",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/src/slice/local_docker.rs",
    "blob": "49ddff903e9c0682599bd5d8015ea6cba4142ebe",
    "ranges": [
      [
        126,
        156
      ]
    ],
    "classification": "shared_explicit_slice_sandbox_policy",
    "rationale": "MP-01/MP-08/MP-11: Common user config controls provider sandbox compatibility on ordinary and broker placements. Broker discovery no longer forces opt-in. False and true project identical settings and production container security arguments; explicit inner-slice isolation remains intact.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "from_config"
      ]
    ],
    "openFindings": [
      "Independent changed-blob review and MP-10 fresh-machine comparison pending."
    ]
  },
  {
    "id": "broker-slice-resource-admission",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        439,
        439
      ],
      [
        565,
        567
      ]
    ],
    "classification": "shared_docker_cpu_admission_policy",
    "rationale": "MP-08/MP-11: Config validation and broker verification use the packaged positive decimal nanocpu policy. Shared cases include fractional CPUs, zero, invalid/nonfinite decimals, precision and native nanocpu overflow. The broker retains independent request validation; deployment does not select CPU grammar.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "validateProvisioner"
      ]
    ],
    "openFindings": [
      "Independent changed-blob review and MP-10 fresh-machine comparison pending."
    ]
  },
  {
    "id": "placement-selected-kernel-dumpability",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/src/slice/local_docker/broker.rs",
    "blob": "2a5ec0a4e08235e74bd5ac1f0ce36b1f73772772",
    "ranges": [
      [
        126,
        159
      ],
      [
        214,
        223
      ]
    ],
    "classification": "shared_kernel_control_memory_policy",
    "rationale": "MP-03/MP-08/MP-11: Every Linux kernel disables process dumpability during early initialization, before runtime credentials load. Failure closes the inherited broker descriptor and fails loudly. Both placements retain the broker capability boundary; separately executed ordinary children reset dumpability for provider diagnostics. Frozen removal dispositions remain historical.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "initialize"
      ],
      [
        "kernel_slice_broker_control",
        "make_process_nondumpable"
      ]
    ],
    "openFindings": [
      "Independent changed-blob review, real provider diagnostics and MP-10 fresh-machine comparison pending."
    ]
  },
  {
    "id": "broker-provisioner-environment-projection",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/src/slice/local_docker/broker.rs",
    "blob": "2a5ec0a4e08235e74bd5ac1f0ce36b1f73772772",
    "ranges": [
      [
        325,
        346
      ],
      [
        599,
        614
      ]
    ],
    "classification": "shared_explicit_typed_slice_tuning",
    "rationale": "MP-08/MP-11: common command construction snapshots documented inherited pids/nofile/free-space settings into typed numeric options and explicitly projects them to both ordinary execution and broker serialization. The broker admits these names with independent matching numeric bounds. Other ambient variables and protected engine controls remain excluded. MP-08/MP-11 raw ordinary controls clear DOCKER_CONTEXT when explicit DOCKER_HOST selects the same engine as builds.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "provisioner_environment"
      ],
      [
        "kernel_slice_broker_control",
        "local_command"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-private-home-archive-dispatch",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/src/slice/local_docker/state.rs",
    "blob": "ebd3aebffc2efd0ecc8ed26d217bd51fee07d56d",
    "ranges": [
      [
        901,
        973
      ]
    ],
    "classification": "shared_slice_private_archive_dispatch",
    "rationale": "The common helper starts before capture. Broker placement delegates HomeArchiveCapture before any tar command; ordinary placement streams stdout into private product state. The outer caller removes the owned helper on either result and retains existing generation publication and disk admission. This dispatch is not a host-release activation path.",
    "anchors": [
      [
        "cleanup_selector",
        "archive_local_docker_home_volume"
      ],
      [
        "kernel_slice_broker_control",
        "archive_local_docker_home_volume_with_helper"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "ordinary-private-home-archive-stream",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/slice/local_docker/home_archive_capture.rs",
    "blob": "b2fd0771360decf5ffdf8da781d4a1c0e9db675f",
    "ranges": [
      [
        1,
        318
      ]
    ],
    "classification": "shared_slice_private_archive_storage",
    "rationale": "Ordinary capture creates a new0600 product file and streams direct Docker tar stdout through a fixed64 KiB buffer without a login shell, helper-layer archive or retained payload. The packaged shared policy supplies the active2 GiB reserve and5-minute inactivity deadline; healthy progress has no total byte/time ceiling. Unix nonblocking reads and Windows readiness distinguish idle from EOF, and unreaped owned producers settle before inode-bound partial cleanup. Outer state dispatch still removes the archive helper.",
    "anchors": [
      [
        "cleanup_selector",
        "capture"
      ],
      [
        "cleanup_selector",
        "remove_created_archive"
      ]
    ],
    "openFindings": [
      "Verification cancellation differs: ordinary backup validation uses blocking filesystem hashing, while managed preverification uses asynchronous progress cancellation. OS-uninterruptible filesystem I/O cannot be settled by these user-space guards. This is unresolved source scope, not a reproduced ordinary-success/managed-failure regression or live-equivalence claim.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "managed-private-home-archive-publication",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        695,
        863
      ]
    ],
    "classification": "shared_slice_private_archive_publication",
    "rationale": "The broker validates state/backup identity and publishes an owned0700 generation containing0600 archive and exact size/digest metadata after fsync. Capture streams directly into that private product state; production supplies no fixed archive size or total-time ceiling. Verification pins archive/generation/metadata descriptors and awaits an asynchronous digest before restoration; removal stays identity/generation-scoped. This is mutable slice state, not host-release activation.",
    "anchors": [
      [
        "cleanup_selector",
        "validateArtifactIdentity"
      ],
      [
        "cleanup_selector",
        "captureHomeArchive"
      ],
      [
        "cleanup_selector",
        "inspectManagedHomeArchive"
      ],
      [
        "cleanup_selector",
        "verifyHomeArchive"
      ],
      [
        "cleanup_selector",
        "removeHomeArchive"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "managed-home-archive-stream-policy",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs",
    "blob": "0f769bb22d0432e74237b942cc2d01b3b4f74ee1",
    "ranges": [
      [
        1,
        116
      ]
    ],
    "classification": "shared_slice_archive_progress_policy",
    "rationale": "The otherwise lexical-blind module imports the same reserve/progress sidecar as ordinary capture. Production capture has null maximum-size and total-deadline defaults, writes chunks directly to an exclusive0600 product file, resets inactivity only on writes, hashes/syncs success, and settles the owned CLI/group before deleting only its partial inode. The close guard retains same-group descendant cleanup after parent exit. Only provision/restore-state with a nonempty saved archive qualify for the separate response-lifetime exemption.",
    "anchors": [
      [
        "cleanup_selector",
        "capturePrivateHomeArchive"
      ],
      [
        "kernel_slice_broker_control",
        "isHomeArchiveRestoreRequest"
      ],
      [
        "cleanup_selector",
        "homeArchiveMetadataMatches"
      ]
    ],
    "openFindings": [
      "Verification cancellation differs: ordinary backup validation uses blocking filesystem hashing, while managed preverification uses asynchronous progress cancellation. OS-uninterruptible filesystem I/O cannot be settled by these user-space guards. This is unresolved source scope, not a reproduced ordinary-success/managed-failure regression or live-equivalence claim.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
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
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
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
    "id": "verified-path1-kernel-launch-and-broker-handoff",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "apps/kernel/src/managed_bootstrap/supervisor.rs",
    "blob": "b7b0be372087afbe570d3fb59fa3fef876da8727",
    "ranges": [
      [
        184,
        295
      ],
      [
        460,
        528
      ]
    ],
    "classification": "deployment_bootstrap_and_kernel_slice_capability",
    "rationale": "MP-08/MP-11: Reinspected verified launch and inherited private-FD handoff on parity3. Provider controls remain scrubbed; response lifetime is owned by the broker producer, not a supervisor timer. Delayed socket and child handoff checks cover this seam; signed deployment and mandatory shutdown remain the only exceptions.",
    "anchors": [
      [
        "managed_only_branch",
        "spawn_kernel_with_handoff"
      ],
      [
        "kernel_slice_broker_control",
        "spawn_with_broker_lease"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
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
    "id": "managed-waiting-room-placement-projection",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "packages/kernel-client/src/waiting-room-runtime-placement.ts",
    "blob": "47d3bb3f5ac2b333b9a09dfb6873edb26a1b8abc",
    "classification": "shared_enrolled_machine_client_projection",
    "rationale": "MP-02/MP-08/MP-11: Ready aliases retain all authorized kernels and resolve their ordinary runtime Machine ID for common launch. Pending/revision-mismatched enrollment remains gated.",
    "anchors": [
      [
        "client_projection",
        "waitingRoomLaunchMachineOptions"
      ],
      [
        "client_projection",
        "waitingRoomLaunchKernelOptions"
      ],
      [
        "client_projection",
        "waitingRoomLaunchPlacement"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "shared-provider-utility-capture-budget",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/local/provider_requests/catalog/probe_capture.rs",
    "blob": "451a629b85da106ac361f0897ee634b087cf1bda",
    "classification": "shared_provider_utility_resource_policy",
    "rationale": "Every provider utility retains one deadline, combined stdout/stderr memory cap and per-stream read budget with platform cleanup. Windows now reuses the common suspended-start job-owned pipe producer; the capture loop still has no managed topology selector. Platform runtime evidence is distinct from this common-loop inspection.",
    "anchors": [
      [
        "cleanup_selector",
        "capture"
      ],
      [
        "cleanup_selector",
        "drain"
      ]
    ]
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
  },
  {
    "id": "hosted-slice-owner-and-token-scope",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
  {
    "id": "per-creation-machine-scoped-slice-ref",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/src/slice/local_docker.rs",
    "blob": "49ddff903e9c0682599bd5d8015ea6cba4142ebe",
    "ranges": [
      [
        368,
        378
      ],
      [
        1077,
        1161
      ],
      [
        1208,
        1210
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
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "ranges": [
      [
        133,
        136
      ],
      [
        1131,
        1214
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
    "id": "path1-broker-prepared-image-admission",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        118,
        170
      ],
      [
        336,
        437
      ],
      [
        439,
        552
      ]
    ],
    "classification": "shared_slice_image_operand_admission",
    "rationale": "The broker consumes the already prepared extension image through existing provisioner policy; it continues rejecting raw caller Dockerfile paths in broker requests. Read-only image inspection, snapshot helper image argument17, and provisioner image/base fields use typed Docker references. Container/volume ownership, matching read-only helper mounts and image commit/removal ownership remain separate checks. The signed helper and sender supply the ordinary extension capability before this boundary.",
    "anchors": [
      [
        "client_projection",
        "ALLOWED_ENVIRONMENT"
      ],
      [
        "managed_only_branch",
        "validateDocker"
      ],
      [
        "managed_only_branch",
        "validateProvisioner"
      ]
    ],
    "openFindings": [
      "Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."
    ]
  },
  {
    "id": "canonical-slice-cloud-recovery-shape",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    path: "apps/kernel/src/slice/local_docker/extension_build.rs",
    blob: "a7f2c8b2536b75f07d0a739d71d6fbdd578eb5f1",
    ranges: [[1, 305]],
    classification: "path1_extension_build_adapter",
    rationale: "Only broker-backed provision with an extension and no saved archive invokes the signed helper. Cache admission precedes opening user context; live caller descriptors pin the selected Dockerfile target and context. The isolated privileged interpreter receives raw environment bytes encoded over stdin and restores settings only after privilege drop. Native-derived bounds replace small managed limits. The prepared image returns to existing broker placement with build policy never. Recovery and saved-state paths retain their existing behavior.",
    anchors: [["managed_only_branch", "prepare"], ["managed_only_branch", "open_dockerfile"], ["managed_only_branch", "capture_client_environment"], ["managed_only_branch", "helper_command"], ["managed_only_branch", "run_helper"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution, independent semantic disposition and signed deployed acceptance remain pending."],
  },
  {
    "id": "path1-pinned-user-extension-helper",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/managed-extension-build.py",
    "blob": "0d1835584a363cd618a3b79db0b049e54dfa4e05",
    "ranges": [
      [
        1,
        523
      ]
    ],
    "classification": "path1_extension_build_adapter",
    "rationale": "MP-03/MP-08/MP-11: Caller/source/cwd/socket pins, UID and capability drop, and caller-bound PID namespaces remain. Build stdout/stderr now stream completely in fixed-memory chunks to private kernel logs; no output-volume failure or discard budget remains. Frozen semantic conclusions do not approve this changed blob. MP-08/MP-11 coordinator engine contract: caller endpoint/builder/startup overrides are ignored with names-only diagnostics; protected rootless socket remains pinned and arbitrary credential-helper settings survive UID drop.",
    "anchors": [
      [
        "managed_only_branch",
        "normalized_image_reference"
      ],
      [
        "managed_only_branch",
        "validate_request"
      ],
      [
        "managed_only_branch",
        "path1_user"
      ],
      [
        "managed_only_branch",
        "pin_invoker"
      ],
      [
        "managed_only_branch",
        "open_source"
      ],
      [
        "managed_only_branch",
        "pin_cwd"
      ],
      [
        "managed_only_branch",
        "daemon_socket_group"
      ],
      [
        "managed_only_branch",
        "pin_socket"
      ],
      [
        "managed_only_branch",
        "signed_digest"
      ],
      [
        "managed_only_branch",
        "project_context"
      ],
      [
        "managed_only_branch",
        "drop_user"
      ],
      [
        "managed_only_branch",
        "namespace_build"
      ],
      [
        "managed_only_branch",
        "main"
      ],
      [
        "cleanup_selector",
        "scratch_lease"
      ],
      [
        "cleanup_selector",
        "capture_build"
      ],
      [
        "cleanup_selector",
        "run_namespace"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-provisioner-extension-cache-and-build",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "ranges": [
      [
        676,
        822
      ],
      [
        1638,
        1665
      ]
    ],
    "classification": "shared_slice_image_build_policy",
    "rationale": "The common build_image path preserves cache/saved-image decisions before check-only exit42 and before opening user extension context. The pinned helper socket selects the local default builder; ordinary Docker selection remains. Actual builds use the shared owned command guard with no total deadline unless the broker alone injects its trusted1200-second child marker. The internal build-image action invokes existing admission/build without container provisioning; recover does not build. MP-08/MP-11: ordinary and managed Buildx use the default builder on the configured slice engine; caller builder/host/startup overrides cannot redirect image production.",
    "anchors": [
      [
        "client_projection",
        "docker_target_arch"
      ],
      [
        "client_projection",
        "run_build_command"
      ],
      [
        "client_projection",
        "docker_build"
      ],
      [
        "client_projection",
        "build_standard_runtime_image"
      ],
      [
        "client_projection",
        "ensure_runtime_base_image"
      ],
      [
        "client_projection",
        "build_image"
      ],
      [
        "client_projection",
        "main"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    id: "typed-docker-image-reference-grammar",
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
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
    sourceCommit: "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    path: "apps/kernel/slice-linux-docker/docker-image-reference.json",
    blob: "455e3bcfbdafe168026ae063627d189e68a31807",
    ranges: [[1, 6]],
    classification: "shared_slice_image_operand_admission",
    rationale: "The exact JSON sidecar binds the image-reference expression, repository-path bound and default runtime base consumed by the Python and JavaScript adapters. Registry/tag/digest grammar applies only to typed image operands, with separate resource mutation ownership. This data inspection does not establish registry availability or live image compatibility.",
    anchors: [["managed_only_branch", "reference"], ["managed_only_branch", "maximumRepositoryPath"], ["managed_only_branch", "defaultBase"]],
    openFindings: ["Fresh ordinary/Path-1 extension execution and independent semantic disposition remain pending."],
  },
  {
    "id": "managed-home-archive-digest-policy",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/slice-linux-docker/managed-home-archive-digest.mjs",
    "blob": "8ac3933cf9b9d4ade377fb9b8e7857c40ce8488d",
    "ranges": [
      [
        1,
        38
      ]
    ],
    "classification": "shared_pinned_preverification_supervisor",
    "rationale": "MP-08/MP-10/MP-11: broker preverification passes the retained archive descriptor as stdin to the same process supervisor as ordinary hashing. It preserves the caller descriptor, validates a bounded digest reply, uses the shared inactivity policy and settles before failure. Actual syscall-stall and pinned-inode replacement fixtures pass; kernel lease cancellation uses the same supervisor.",
    "anchors": [
      [
        "cleanup_selector",
        "digestPinnedHomeArchive"
      ],
      [
        "cleanup_selector",
        "validateProgressTimeout"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-home-archive-policy-data",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/home-archive-policy.json",
    "blob": "1d54fe7e4eca155a39afdb9a30939465a7037f5a",
    "ranges": [
      [
        1,
        1
      ]
    ],
    "classification": "shared_slice_archive_progress_policy",
    "rationale": "The immutable sidecar supplies schema1, minimumFreeBytes2147483648 and progressTimeoutMs300000. Rust capture/disk admission and Node capture/digest use these same values; the Python second-reader guard validates the same packaged sidecar. These are shared active reserve/inactivity limits, without a32 GiB cap or healthy-stream total deadline.",
    "anchors": [
      [
        "cleanup_selector",
        "schemaVersion"
      ],
      [
        "cleanup_selector",
        "minimumFreeBytes"
      ],
      [
        "cleanup_selector",
        "progressTimeoutMs"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "home-archive-broker-response-lifetime",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/src/slice/local_docker/broker.rs",
    "blob": "2a5ec0a4e08235e74bd5ac1f0ce36b1f73772772",
    "ranges": [
      [
        208,
        212
      ],
      [
        251,
        317
      ]
    ],
    "classification": "shared_owned_operation_response_lifetime",
    "rationale": "MP-08/MP-10/MP-11: all broker response waits have no placement-selected total deadline. Request writes retain a30-second transport bound. The caller read state is restored on success and decode failures; ownership/cancellation is enforced at the broker producer. Archive and nonarchive delayed responses are tested.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "configure_stream_deadlines"
      ],
      [
        "kernel_slice_broker_control",
        "execute_with_disk_evidence"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-home-archive-disk-reserve-admission",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/slice/local_docker/disk_admission.rs",
    "blob": "c97438de3b8ce2dd78d049f25e28be42a151a4a1",
    "ranges": [
      [
        14,
        20
      ],
      [
        46,
        199
      ]
    ],
    "classification": "shared_slice_archive_disk_admission",
    "rationale": "Common snapshot admission estimates home/entries/writable-layer overhead and accounts for shared versus separate storage. Both placements use the capture sidecar minimum reserve and the same admission path. Scoped ranges include process/engine admission handoff but exclude platform lock internals and storage-measurement subprocesses.",
    "anchors": [
      [
        "cleanup_selector",
        "evaluate_slice_snapshot_disk_admission"
      ],
      [
        "cleanup_selector",
        "with_slice_snapshot_disk_admission"
      ],
      [
        "cleanup_selector",
        "validate_slice_snapshot_disk_admission"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "shared-windows-pipe-producer-ownership",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/io/windows_pipe_process.rs",
    "blob": "9ee7448312dbc063dbc5fb6cf37bf8299b8b9ac4",
    "ranges": [
      [
        1,
        158
      ]
    ],
    "classification": "shared_windows_pipe_resource_policy",
    "rationale": "Common Windows archive/provider pipe producers start suspended, join an owned job before resume, and settle that job/child through owned handles. Pipe readiness distinguishes empty-open from broken-pipe EOF. This platform adapter has no ordinary/managed selector; Windows execution/ABI evidence is separate from source inspection.",
    "anchors": [
      [
        "cleanup_selector",
        "readiness"
      ],
      [
        "cleanup_selector",
        "Process"
      ],
      [
        "cleanup_selector",
        "spawn"
      ],
      [
        "cleanup_selector",
        "stop"
      ],
      [
        "cleanup_selector",
        "resume"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "ordinary-backup-verification-cancellation-scope",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/src/slice/local_docker/state.rs",
    "blob": "ebd3aebffc2efd0ecc8ed26d217bd51fee07d56d",
    "ranges": [
      [
        178,
        264
      ],
      [
        975,
        977
      ]
    ],
    "classification": "shared_pinned_preverification_supervisor",
    "rationale": "MP-08/MP-10/MP-11: ordinary preverification delegates a retained regular-file descriptor to the same Python progress/cancellation supervisor as the broker. Size/digest derive from the opened inode and metadata is checked after hashing. Both placements use the packaged inactivity policy,64KiB reads, parent-death supervision and owned process settlement. Stalled reads before/after progress are tested; uninterruptible OS I/O cannot be made cancellable in user space.",
    "anchors": [
      [
        "cleanup_selector",
        "validate_local_docker_slice_backup"
      ],
      [
        "cleanup_selector",
        "file_sha256"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-provisioner-command-ownership",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "ranges": [
      [
        1,
        41
      ],
      [
        202,
        239
      ],
      [
        276,
        293
      ],
      [
        1596,
        1636
      ]
    ],
    "classification": "shared_slice_control_process_policy",
    "rationale": "Ordinary and broker-backed noninteractive Docker controls use the same Python-owned command wrapper; interactive login retains its TTY. Named explicit commands select the executable/optional helper socket without a login-shell fallback. Stop/destroy require successful inventories before absence classification, and volume absence is accepted only for exact requested single diagnostics. Shared control deadlines do not establish complete cross-placement parity. MP-08/MP-11: privileged Bash startup ignores ambient hooks; image builds use the configured slice engine, default builder and explicit endpoint selection with names-only override diagnostics.",
    "anchors": [
      [
        "cleanup_selector",
        "docker"
      ],
      [
        "cleanup_selector",
        "run_guarded_command"
      ],
      [
        "cleanup_selector",
        "run_with_timeout"
      ],
      [
        "cleanup_selector",
        "run_with_file_stdin_timeout"
      ],
      [
        "cleanup_selector",
        "volume_inspect_reports_not_found"
      ],
      [
        "cleanup_selector",
        "stop_container"
      ],
      [
        "cleanup_selector",
        "destroy_container"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "shared-saved-home-restore-stream-and-identity",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "ranges": [
      [
        294,
        403
      ]
    ],
    "classification": "shared_slice_private_archive_restore",
    "rationale": "Before creating/restoring a missing home volume, the shared second-reader digest uses the packaged inactivity policy and retains the archive identity/initialization-token labels. Ordinary restore streams the existing product archive via stdin to Docker exec -i tar; managed placement uses the broker-pinned read-only generation mount. Positional /bin/sh extraction avoids login profiles and helper-layer archive copies. The owned restore helper is removed on either outcome; only a proven newly-created failed volume is removed.",
    "anchors": [
      [
        "cleanup_selector",
        "saved_home_archive_identity"
      ],
      [
        "cleanup_selector",
        "restore_saved_home_volume"
      ],
      [
        "cleanup_selector",
        "prepare_home_volume"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "broker-control-settlement",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        911,
        980
      ],
      [
        1248,
        1264
      ],
      [
        1391,
        1406
      ]
    ],
    "classification": "managed_slice_control_process_adapter",
    "rationale": "Broker preflight Docker and mount utilities use the bounded20-second SIGKILL control wrapper, with30 seconds for exact owned-container stop. Timeout/signal/invalid inspection cannot be mistaken for absence or a usable mount; handle publication rechecks descriptor inode identity. This adapter does not classify arbitrary daemon errors as missing objects. The actual stop/second-reader fixtures show settlement and same-process followup in private namespaces, not live daemon or full rollback acceptance.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "spawnControl"
      ],
      [
        "kernel_slice_broker_control",
        "handleIsMountpoint"
      ],
      [
        "kernel_slice_broker_control",
        "unmountHandle"
      ],
      [
        "kernel_slice_broker_control",
        "publishHandle"
      ],
      [
        "kernel_slice_broker_control",
        "inspectContainerMounts"
      ],
      [
        "kernel_slice_broker_control",
        "requireExactContainerMounts"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "broker-build-duration-limit",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "ranges": [
      [
        1563,
        1570
      ],
      [
        1673,
        1685
      ]
    ],
    "classification": "shared_owned_operation_lifetime",
    "rationale": "MP-08/MP-10/MP-11: no placement-selected20/21-minute total command cap or1200-second build injection remains. Broker commands use the same Python process-group owner as ordinary provisioning; lease close cancels producers before prepared handles are released. Healthy and silent producers and signal-resistant descendants are covered locally; per-step control deadlines remain shared.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "spawnBounded"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending."
    ]
  },
  {
    "id": "shared-provisioner-command-guard",
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304",
    "path": "apps/kernel/slice-linux-docker/slice-command-guard.py",
    "blob": "a164518800a9a037d556975ea28a58636209d78e",
    "ranges": [
      [
        1,
        262
      ]
    ],
    "classification": "shared_slice_control_and_digest_progress_policy",
    "rationale": "The shared supervisor passes its original identity into a still-occupied worker group anchor, forwards bounded chunks without payload retention, and settles descendants before returning command status. Cancellation/parent loss kills only its owned group. Archive and runtime-source digest workers use fixed chunks plus shared inactivity; result IPC retains ownership through cleanup. Ordinary/extension builds may choose owned unbounded mode. OS-uninterruptible I/O remains outside user-space settlement guarantees.",
    "anchors": [
      [
        "cleanup_selector",
        "progress_timeout_seconds"
      ],
      [
        "cleanup_selector",
        "stop_orphaned_anchor"
      ],
      [
        "cleanup_selector",
        "finish_worker"
      ],
      [
        "cleanup_selector",
        "command_worker"
      ],
      [
        "cleanup_selector",
        "digest_worker"
      ],
      [
        "cleanup_selector",
        "source_digest_worker"
      ],
      [
        "cleanup_selector",
        "run_owned"
      ],
      [
        "cleanup_selector",
        "main"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts, ordinary-versus-Path-1 fresh-machine comparison and cleanup remain pending.",
      "Independent semantic disposition, credential-preserving live comparison and full MP01–MP11 acceptance remain pending."
    ]
  },
  {
    "id": "m20-owned-fallback-image-cleanup",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/cli/scripts/lib/browser-state-drill-cleanup.mjs",
    "blob": "9c0f3f9fb2638c5b6314649c18d3e6c93c80f7d4",
    "ranges": [
      [
        1,
        55
      ]
    ],
    "classification": "validation_owned_image_cleanup",
    "rationale": "Synthetic M20 fallback candidates exclude the baseline, then require exact SliceRecord slice/owner-Kernel/owner-Machine labels before removing the inspected immutable image ID and verifying absence. Foreign or incomplete image ownership is preserved; missing drill identity and inspection/removal errors remain unresolved. This is validation resource cleanup, not a runtime authority or live cleanup claim.",
    "anchors": [
      [
        "cleanup_selector",
        "browserStateCleanupFailure"
      ],
      [
        "cleanup_selector",
        "cleanupBrowserStateImages"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "m20-fallback-image-cleanup-callers",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/cli/scripts/live-docker-slice-browser-state-drill.mjs",
    "blob": "d94d169c1a9003e6a6d77488f22aee43664915cf",
    "ranges": [
      [
        1025,
        1127
      ]
    ],
    "classification": "validation_owned_image_cleanup",
    "rationale": "The M20 cleanup calls the shared three-label immutable-ID selector for both newly discovered state and restore-rollback fallback inventories. Known explicit saved-state/backup identities retain their original path. Unresolved owned/inventory cleanup fails the drill while preserved foreign images are not treated as this run leaks. Source inspection does not prove a live save/restore campaign.",
    "anchors": [
      [
        "cleanup_selector",
        "cleanup"
      ]
    ],
    "openFindings": [
      "Independent semantic disposition, credential-preserving live comparison and full MP01\u2013MP11 acceptance remain pending."
    ]
  },
  {
    "id": "shared-owned-broker-command",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/slice-linux-docker/managed-broker-command.mjs",
    "blob": "00e377629d5343a013bee83044192dffbaff27ac",
    "ranges": [
      [
        1,
        55
      ]
    ],
    "classification": "shared_owned_streaming_command",
    "rationale": "MP-08/MP-10/MP-11: common Python owner supervises broker runtime/build producers without a total deadline, settles cancellation, writes complete private logs and bounds only the response tail. Real output and cancellation regressions pass; this observation is not acceptance.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "runBrokerCommand"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts and fresh-machine comparison remain pending."
    ]
  },
  {
    "id": "ordinary-pinned-archive-verify",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/src/slice/local_docker/home_archive_verify.rs",
    "blob": "383df4fa45fd7a67fdd571650cde463f29ac8440",
    "ranges": [
      [
        1,
        101
      ]
    ],
    "classification": "shared_pinned_preverification_supervisor",
    "rationale": "MP-08/MP-10/MP-11: ordinary archive hashing pins a regular file and delegates its retained descriptor to the shared progress supervisor, with before/after metadata checks. This scope retains cancellation and inode identity review obligations.",
    "anchors": [
      [
        "cleanup_selector",
        "digest"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts and fresh-machine comparison remain pending."
    ]
  },
  {
    "id": "common-slice-tuning-options",
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c",
    "path": "apps/kernel/src/slice/local_docker/tuning.rs",
    "blob": "15db8bf32b354e16834c6075556f1049a1a44714",
    "ranges": [
      [
        1,
        69
      ]
    ],
    "classification": "shared_explicit_typed_slice_tuning",
    "rationale": "MP-08/MP-11: common typed option capture validates inherited pids/nofile/free-space settings and explicitly projects them for ordinary commands and broker request serialization. Protected engine and broker controls are never admitted as inherited tuning.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "project"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11 independent changed-blob review, signed aggregate artifacts and fresh-machine comparison remain pending."
    ]
  },
  // MP-11 mp11b: exact scopes from the high-risk out-of-scope review.
  {
    "id": "mp11b-ready-machine-kernel-projection",
    "sourceCommit": "d0638173d10e1fa6040741d72da455d9847b4322",
    "path": "apps/web/src/ui/waiting-room-runtime-placement.ts",
    "blob": "5eb52b577ba4a3f85925e1f543b4b339167438b0",
    "ranges": [
      [
        47,
        66
      ],
      [
        103,
        129
      ]
    ],
    "classification": "shared_enrolled_machine_client_projection",
    "rationale": "MP-08/MP-11: Ordinary runtime Machines remain selectable. A ready environment alias exposes the same account-authorized kernel list as that Machine; deployment readiness still gates pending aliases.",
    "anchors": [
      [
        "client_projection",
        "waitingRoomMachineOptions"
      ],
      [
        "client_projection",
        "waitingRoomKernelsForSelectedMachine"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "mp11b-ready-machine-home-readiness",
    "sourceCommit": "d0638173d10e1fa6040741d72da455d9847b4322",
    "path": "apps/web/src/ui/waiting-room-launch-readiness.ts",
    "blob": "e3ad8eb2976de03c725bdc2bd6a33b2479aa476f",
    "ranges": [
      [
        101,
        110
      ]
    ],
    "classification": "shared_selected_kernel_readiness",
    "rationale": "MP-08/MP-11: Ready alias admission compares the connected target with the common selected authorized kernel, then applies ordinary heartbeat/provider readiness. The bootstrap home is not forced after enrollment.",
    "anchors": [],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "mp11b-shared-host-unit",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/chariox-managed-bootstrap.service",
    "blob": "902948442e4ecc771b86e56be1cfa79e83ea583e",
    "ranges": [
      [
        1,
        58
      ]
    ],
    "classification": "shared_host_isolation",
    "rationale": "MP-01/MP-11: This unit explicitly selects shared_host and the Bubblewrap isolation marker. Its hardening belongs to that separate topology; Path-1 preparation explicitly chooses chariox-path1-managed-bootstrap.service. This conclusion covers the checked-in unit, not installed overrides.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-broker-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/chariox-slice-broker.service",
    "blob": "8195754884b2c17609ce7812b914c49b8935c6a3",
    "ranges": [
      [
        1,
        36
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-01/MP-03/MP-11: Hardening applies to this independently launched slice control/engine service principal, not the Path-1 kernel service or its provider children. Service ordering and Wants are not process ancestry. This does not approve downstream broker/engine runtime policy; existing slice differences remain removal required.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-rootless-lifecycle-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/chariox-rootless-docker.service",
    "blob": "8aae4b3f4809d3bc3d165fdaa137480927edee3b",
    "ranges": [
      [
        1,
        41
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-01/MP-03/MP-11: Hardening applies to this independently launched slice control/engine service principal, not the Path-1 kernel service or its provider children. Service ordering and Wants are not process ancestry. This does not approve downstream broker/engine runtime policy; existing slice differences remain removal required.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-rootless-engine-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/chariox-rootless-engine.service",
    "blob": "4a146d543dd669d493cb4e446a4269c7939c3c69",
    "ranges": [
      [
        1,
        17
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-01/MP-03/MP-11: Hardening applies to this independently launched slice control/engine service principal, not the Path-1 kernel service or its provider children. Service ordering and Wants are not process ancestry. This does not approve downstream broker/engine runtime policy; existing slice differences remain removal required.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-quota-allocator-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/chariox-slice-disk-quota-allocator.service",
    "blob": "8e6bee212317d8beea536916e12002ddf7df934c",
    "ranges": [
      [
        1,
        39
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-01/MP-03/MP-11: Hardening applies to this independently launched slice control/engine service principal, not the Path-1 kernel service or its provider children. Service ordering and Wants are not process ancestry. This does not approve downstream broker/engine runtime policy; existing slice differences remain removal required.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-data-volume-admission-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/chariox-data-volume-admission.service",
    "blob": "cfadfdfca462e4752ea7a69d90351227da145881",
    "ranges": [
      [
        1,
        30
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-11: The dedicated oneshot checks deployment data-volume admission before separately launched services. Its own filesystem/network restrictions do not become the provider process sandbox. The checked-in ExecStart runs admission, not a provider.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-image-builder-cleanup-service",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/chariox-image-builder-cleanup@.service",
    "blob": "cf4006dbd65c3a6b4a25c2c543f9b390ed1aa478",
    "ranges": [
      [
        1,
        19
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-11: This root oneshot expires the exact temporary release image-builder receipt. Its sandbox affects the deployment cleanup helper only and does not select a kernel/provider launch.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-scm-home-selection",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/managed_context/scm.rs",
    "blob": "ac4c45f2afb93daefd8749f9f06cdea4642e7562",
    "ranges": [
      [
        108,
        169
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-04/MP-08/MP-11: Inventory uses process HOME/PATH/XDG/GH settings when CHARIOX_MANAGED_PROVIDER_HOME is absent. Both Path-1 launchers remove that selector; the alternate provider home belongs to shared-host/slice isolation. managed_target is the explicit credential-transfer destination constructor, also used for ordinary targets, not a Path-1-only runtime branch.",
    "anchors": [
      [
        "managed_only_branch",
        "source_from_process"
      ],
      [
        "managed_only_branch",
        "managed_target"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-pty-process-marker",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/pty/manager.rs",
    "blob": "1560ca2dcbc15dae6c0240c477233eb67b8b2867",
    "ranges": [
      [
        261,
        286
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: Every newly spawned kernel-owned provider run gets CHARIOX_MANAGED_PROVIDER_PROCESS=1 after the common discovery scrub. The method has no machine-placement check; managed here describes Chariox ownership of the PTY process, not Path-1 placement.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-opencode-discovery-namespace-adapter",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/provider/opencode/discovery.rs",
    "blob": "a466ebb02f58b10e08af75464248c86dd792e983",
    "ranges": [
      [
        19,
        91
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-01/MP-08/MP-11: Read-only OpenCode discovery modifies namespace arguments only when the existing --setenv isolation marker is present. Unwrapped ordinary and Path-1 launches return unchanged. The generic discovery configuration is created under the common preparation HOME on both placements.",
    "anchors": [
      [
        "managed_only_branch",
        "apply_to_launch_args"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-preparation-namespace-adapters",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/provider/managed_isolation.rs",
    "blob": "37ae2123aaea0e9eaf82374a3ef45018a69cc017",
    "ranges": [
      [
        211,
        314
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-01/MP-08/MP-11: Preparation-HOME rewriting and validation inspect the actual isolation-marker arguments. An unwrapped Path-1/ordinary run is a no-op and passes the preparation-home check; only explicit sandbox topologies rewrite bind mounts. No managed-machine selector appears in these branches.",
    "anchors": [
      [
        "managed_only_branch",
        "apply_preparation_home_to_managed_launch"
      ],
      [
        "managed_only_branch",
        "managed_launch_has_preparation_home"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-provider-reported-path-adapter",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/provider/managed_isolation.rs",
    "blob": "37ae2123aaea0e9eaf82374a3ef45018a69cc017",
    "ranges": [
      [
        456,
        513
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-02/MP-08/MP-11: Without the actual isolation-marker arguments, reported provider paths return unchanged. Namespace bind translation is confined to explicit isolated runs. It is not a managed-machine workspace filter.",
    "anchors": [
      [
        "protected_path_filter",
        "provider_reported_path_on_kernel"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-runtime-environment-inputs",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/config/env_loader.rs",
    "blob": "b7707033b9d9fe03f3ad911ddac38e6f119bc5ec",
    "ranges": [
      [
        104,
        113
      ],
      [
        180,
        205
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: Publication control state and machine-owned slice recovery/public-key fields are loaded through common DaemonConfig for ordinary and Path-1 kernels. They identify an explicit publication/slice role, without selecting different provider execution or workspace admission.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-project-validation-control-scrub",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/runtime/state/project_environment_setup_validation.rs",
    "blob": "59708026b354c26305d248016dedd1ca47f7fd03",
    "ranges": [
      [
        153,
        221
      ],
      [
        457,
        532
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: Recipe application and validation use one environment constructor on both local ordinary and leased workers. It removes kernel/account/Git controls, retains project/toolchain inputs, and applies the durable preparation HOME/PATH. These managed-selector names are removed controls, not placement-selected project restrictions.",
    "anchors": [
      [
        "managed_only_branch",
        "worker_validation_environment_with_home_and_definition"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-installer-exact-control-repair",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/install-image.sh",
    "blob": "c3d9d546c6a5d771581a0094338eaf9ab0efea34",
    "ranges": [
      [
        232,
        279
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-03/MP-07/MP-11: Installation repairs only named bootstrap files and dedicated managed/disposable-worker/kernels control trees. It does not recursively chmod their shared state_root parent. This is signed deployment control-file ownership, not a workspace path allowlist.",
    "anchors": [
      [
        "protected_path_filter",
        "repair_root_control_file"
      ],
      [
        "protected_path_filter",
        "repair_root_control_tree"
      ],
      [
        "protected_path_filter",
        "repair_root_control_state"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-fresh-rebuild-process-negative-guards",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/managed_bootstrap/freshness.rs",
    "blob": "d9d5c11b1b9189cbd4fbd2826a2845bfe04e255c",
    "ranges": [
      [
        507,
        583
      ]
    ],
    "classification": "negative_guard",
    "rationale": "MP-01/MP-07/MP-10/MP-11: Rebuild admission rejects observable Bubblewrap and retired runtime processes and a malformed ancestor chain. The bwrap strings are negative residue checks, not executable launch selection. This source guard alone does not establish a residue-free rebuild.",
    "anchors": [
      [
        "release_activation",
        "validate_process_observations"
      ],
      [
        "release_activation",
        "is_bubblewrap"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-inner-provider-bwrap-launcher",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/docker/managed-provider-bwrap.sh",
    "blob": "88741c2c0ebb04f8c2c8ab75a884b87e77b444bc",
    "ranges": [
      [
        1,
        17
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-01/MP-11: This packaged inner-slice launcher chooses setpriv only under a rootless UID map and always applies the packaged seccomp descriptor. It is installed in the Docker slice image; Path-1 host providers use the unwrapped common adapter.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-slice-runtime-provider-home",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/slice-linux-docker/docker/start-runtime.sh",
    "blob": "195ce5230578b6ed93068918d0d7f9ce9227c73e",
    "ranges": [
      [
        26,
        42
      ]
    ],
    "classification": "inner_docker_slice_isolation",
    "rationale": "MP-04/MP-11: Provider HOME/probe variables initialize the explicit headed Docker-slice runtime, with private slice-local directories. Both ordinary and managed host placements use this same inner runtime script. This does not approve the separately flagged placement-selected sandbox option.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-image-topology-admission",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "deploy/managed-kernel/prepare-hetzner-image.sh",
    "blob": "f9c8f12c97fdb7e66df523615e4bf7f30daf8ec8",
    "ranges": [
      [
        211,
        229
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-01/MP-07/MP-11: Preparation explicitly selects the Path-1 service or the shared-host service, rejects an absent/unknown topology and verifies an external public builder pin for Path 1. This is deployment selection, not a managed-only provider restriction.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-bootstrap-api-configuration",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/api/src/managed-environments/bootstrap-config.ts",
    "blob": "ebed439b48604dbbea6e0ea539af524ecee1b57c",
    "ranges": [
      [
        1,
        8
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-11: This Cloud helper selects the bootstrap API origin and rejects missing production configuration when the infrastructure manager is configured. It is Cloud enrollment configuration, not a runtime terminal/proxy or provider path.",
    "anchors": [
      [
        "release_activation",
        "managedKernelCloudApiUrl"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-placement-catalog-environment",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/api/src/managed-environments/placement-catalog.ts",
    "blob": "0bb1a248af873f0739090dd20524136c0dc6b667",
    "ranges": [
      [
        25,
        97
      ],
      [
        120,
        207
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-09/MP-11: Environment variables form the Cloud compute/profile/release catalog and server-authoritative CREATE route. The release quiescence gate protects mandatory shutdown support. No provider runtime, cwd, prompt or history behavior is selected here after enrollment.",
    "anchors": [
      [
        "release_activation",
        "configuredManagedEnvironmentPlacementCatalog"
      ],
      [
        "release_activation",
        "parsePlacementSelections"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-candidate-placement-environment",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/api/src/managed-environments/configured-operator-candidate-admission.ts",
    "blob": "bacffca9218f533a1139c6a0b1ea8610b82e1aa2",
    "ranges": [
      [
        1,
        51
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-11: Default-off operator candidate admission builds one pinned deployment route and validates its bounded admission window without publishing it in the public catalog. It does not grant runtime capabilities or introduce another kernel execution model.",
    "anchors": [
      [
        "release_activation",
        "configuredOperatorCandidateAdmission"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-quiescence-release-approval",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/api/src/managed-environments/auto-stop-quiescence-release-policy.ts",
    "blob": "f425167d5b1ddbcaed35b8283d6e750aac616c8f",
    "ranges": [
      [
        1,
        23
      ]
    ],
    "classification": "required_automatic_shutdown",
    "rationale": "MP-09/MP-11: The approved immutable-release digest list prevents enabling Cloud shutdown against unreviewed quiescence code. The selector governs mandatory shutdown compatibility, one of the two permitted differences; it does not select a provider sandbox.",
    "anchors": [
      [
        "automatic_shutdown_selector",
        "configuredQuiescenceReleaseDigests"
      ],
      [
        "automatic_shutdown_selector",
        "requireQuiescenceRelease"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-worker-placement-lifetime-policy",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/api/src/disposable-workers/placement-policy.ts",
    "blob": "4aa94f2b02cf027beff46a1981a701866eb6c393",
    "ranges": [
      [
        1,
        79
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-09/MP-11: Cloud worker policy restricts allocation compute/architecture/concurrency, binds the same reviewed managed placement/storage and requires a bounded lifetime. These are CREATE and required shutdown/cost controls outside the worker, not provider filesystem or session policy.",
    "anchors": [
      [
        "release_activation",
        "configuredDisposableWorkerPlacementPolicy"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-activity-registration-selection",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/kernel/src/runtime/managed_kernel_activity.rs",
    "blob": "c194aff6b1c348d518f9e4854bc22defa1abcbd1",
    "ranges": [
      [
        74,
        122
      ]
    ],
    "classification": "required_automatic_shutdown",
    "rationale": "MP-09/MP-11: Reporter selection uses the confirmed machine or disposable-worker activity receipt and machine credential solely for the mandatory Cloud activity/shutdown lifecycle. Ordinary unregistered kernels have no reporter. Source selection does not prove idle timing or every live shutdown trigger.",
    "anchors": [
      [
        "automatic_shutdown_selector",
        "from_runtime"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-publication-route-path-filter",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/server/src/publication-agent-app-effects.ts",
    "blob": "901d042896a86a189454443e4c1bda057f6c3622",
    "ranges": [
      [
        167,
        196
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-02/MP-11: protected_paths match URL asset overlay routes in an Agent App publication, not Unix workspaces. Every publication uses the same route policy; no managed-machine/Path-1 selector exists in the helper.",
    "anchors": [
      [
        "protected_path_filter",
        "routeAllowsOverlay"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-publication-route-filter-schema",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/server/src/publication-agent-app-schema.ts",
    "blob": "ff5b4032af14e220d34bab413921f4d416a98ff9",
    "ranges": [
      [
        100,
        135
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-02/MP-11: allowed/protected_paths validate user-authored publication route patterns and action IDs. They are shared HTTP asset manipulation policy, not provider cwd or repository restrictions.",
    "anchors": [
      [
        "protected_path_filter",
        "validateRoute"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-release-archive-service-mount",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "deploy/openship/cloud-release.compose.yml",
    "blob": "bcb3933e17543c24410c9710f7f9ac3d890a5cf7",
    "ranges": [
      [
        63,
        67
      ]
    ],
    "classification": "allowed_release_deployment",
    "rationale": "MP-07/MP-11: This Cloud service environment/mount reads signed release archives for authorized in-place upgrades. The volume is a deployment artifact source, not a runtime terminal/history/provider-state path.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-shared-live-sync-mode-projection",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "packages/kernel-client/src/workspace-live-sync-mode.ts",
    "blob": "d19e756e1c7c1aafa1d8815ea94674dbb47cd60d",
    "ranges": [
      [
        1,
        69
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: managed is an explicit user-selected Workspace Live Sync mode, alongside tracked/off, and maps to the common protocol value. The formatter has no managed-machine selector and states the selected-workspace scope.",
    "anchors": [
      [
        "client_projection",
        "parseWorkspaceLiveSyncModeCommand"
      ],
      [
        "client_projection",
        "formatWorkspaceLiveSyncModeLabel"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-web-live-sync-mode-projection",
    "sourceCommit": "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2",
    "path": "apps/web/src/terminal/workspace-live-sync-mode.ts",
    "blob": "5e125bd795151913a7baaae4dd29749fdc44e77c",
    "ranges": [
      [
        1,
        46
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: Web normalizes and labels the same user-selected managed/tracked/unrestricted Live Sync modes. These are shared session settings, not managed placement or an additional parity exception.",
    "anchors": [
      [
        "client_projection",
        "normalizeWorkspaceLiveSyncMode"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-ios-live-sync-mode-command",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/ios/CharioxPackage/Sources/CharioxFeature/State/CharioxAppModelCommands.swift",
    "blob": "76084e6866b37607d0c30b2122e84597c1821ea6",
    "ranges": [
      [
        150,
        175
      ]
    ],
    "classification": "ordinary_path1_behavior",
    "rationale": "MP-08/MP-11: Swift dispatches explicit /workspace sync managed/tracked/off choices to the same session mode command. managed is user-selected Live Sync behavior, not a Cloud-machine branch. No native build/live parity is claimed.",
    "anchors": [],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-runtime-projection-fixture",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "packages/kernel-client/src/session-runtime-projection.test-support.ts",
    "blob": "e196b7a3d2fe1ca8af77ce037f667fc3c0f93922",
    "ranges": [
      [
        17,
        32
      ]
    ],
    "classification": "test_evidence",
    "rationale": "MP-11: This test-support constructor returns synthetic session-1 Live Sync status with empty force_excludes. It has no environment/topology branch and does not install runtime path protection. Its lexical managed value is fixture data.",
    "anchors": [
      [
        "client_projection",
        "workspaceLiveSyncStatus"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-soak-sandbox-root-guard",
    "sourceCommit": "686ec57d5e46cdd46e723155b7eb89f6f25202a2",
    "path": "apps/cli/scripts/lib/browser-computer-soak-runtime.mjs",
    "blob": "d2041162b4ed01208f69dd2e91cbd888ec093a54",
    "ranges": [
      [
        853,
        862
      ]
    ],
    "classification": "negative_guard",
    "rationale": "MP-01/MP-11: This validation preflight rejects root Chromium on every placement. The inherited isolation marker changes only the failure advice, not production browser launch or runtime permissions. The source conclusion does not count as an executed soak.",
    "anchors": [
      [
        "managed_env_selector",
        "assertSandboxCapableChromiumIdentity"
      ]
    ],
    "openFindings": [
      "MP-11: Source-only conclusion; effective host policy, live comparison and program acceptance remain open."
    ]
  },
  {
    "id": "mp11b-ready-machine-forced-preparation",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "apps/cli/src/waiting-room-controller.ts",
    "blob": "be39b058449d1d81ff7ff48a95b19d92dba20a23",
    "ranges": [
      [
        396,
        424
      ]
    ],
    "classification": "shared_ready_machine_launch",
    "rationale": "MP-02/MP-08/MP-11: Ready environments return no managed-preparation selection. The normal client pivot and session creation retain cwd/worktree/Project/worker/slice inputs; pending deployment still uses preparation.",
    "anchors": [
      [
        "client_projection",
        "waitingRoomManagedLaunchSelection"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "mp11b-ready-machine-transfer-workspace-reset",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "apps/cli/src/waiting-room-managed-environment-launch-controller.ts",
    "blob": "425614771545f3ef80e7ff14c1f04136e9336b64",
    "ranges": [
      [
        245,
        275
      ],
      [
        304,
        340
      ]
    ],
    "classification": "initial_enrollment_transfer_defaults",
    "rationale": "MP-02/MP-08/MP-11: This unchanged transfer-target constructor is now reachable only from initial or pending enrollment preparation. Ready launch bypasses it at waitingRoomManagedLaunchSelection. Its defaults no longer replace ordinary ready-session choices.",
    "anchors": [
      [
        "client_projection",
        "workspacePath"
      ],
      [
        "client_projection",
        "managedProjectPreparation"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "mp11b-ready-machine-launch-choice-reset",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "apps/cli/src/cli-waiting-room-composition.ts",
    "blob": "ab33e3ceb9b296663ff7d7365c1ab89a6a26e3af",
    "ranges": [
      [
        547,
        632
      ]
    ],
    "classification": "initial_enrollment_transfer_defaults",
    "rationale": "MP-02/MP-08/MP-11: The unchanged prepared-launch composition applies transfer defaults only after initial/pending enrollment. Ready launch carries no managedEnvironment input and enters common activation directly. This caller restriction is tested; unchanged historical findings remain frozen.",
    "anchors": [
      [
        "client_projection",
        "prepareManagedSessionLaunch"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "mp11b-ready-machine-execution-worker-reset",
    "sourceCommit": "d0638173d10e1fa6040741d72da455d9847b4322",
    "path": "apps/web/src/terminal/waiting-room-managed-environment-launch-controller.ts",
    "blob": "51c98a6e90f1e48ca1097bf6ac172f6b70caa95b",
    "ranges": [
      [
        119,
        149
      ],
      [
        183,
        243
      ]
    ],
    "classification": "shared_ready_machine_launch",
    "rationale": "MP-02/MP-08/MP-11: Initial detail readiness selects the common existing-ready route. It connects the chosen authorized kernel, keeps workspace/Project/worker/slice choices, and clears the obsolete transfer setup target. It skips getLaunchTarget/applyReadyTarget; initial deployment still receives transfer defaults.",
    "anchors": [
      [
        "client_projection",
        "attempt"
      ],
      [
        "client_projection",
        "selectedKernelId"
      ]
    ],
    "openFindings": [
      "MP-08/MP-10/MP-11: Implementation-source disposition only; independent exact-head review, signed aggregate, fresh ordinary/Path-1 comparison and cleanup acceptance remain open. Historical removal findings are not rewritten."
    ]
  },
  {
    "id": "parity3-supervisor-broker-response-owner",
    "sourceCommit": "ead7d3d89a9d80377a2f9f60513715b3d3bd9780",
    "path": "apps/kernel/src/managed_bootstrap/supervisor.rs",
    "ranges": [
      [
        41,
        98
      ],
      [
        578,
        642
      ]
    ],
    "classification": "shared_owned_operation_lifetime",
    "rationale": "MP-08/MP-11: Supervisor backend read timeout is explicitly cleared, including inherited timeout state. Request writes retain30seconds. Framed EOF/error loses the lease; normal/archive/build delayed responses retain it. This closes the upstream timer seam left by the kernel-socket correction.",
    "anchors": [
      [
        "kernel_slice_broker_control",
        "configure_broker_stream_deadlines"
      ],
      [
        "kernel_slice_broker_control",
        "proxy_kernel_broker"
      ]
    ],
    "blob": "b7b0be372087afbe570d3fb59fa3fef876da8727",
    "openFindings": [
      "MP-08/MP-10/MP-11: independent exact-head source review and signed/fresh-machine acceptance remain open."
    ]
  },
  {
    "id": "parity3-interactive-slice-protocol-admission",
    "sourceCommit": "d0638173d10e1fa6040741d72da455d9847b4322",
    "path": "apps/web/src/kernel/slice-requests.ts",
    "ranges": [
      [
        148,
        163
      ]
    ],
    "classification": "shared_protocol_feature_admission",
    "rationale": "MP-08/MP-11: Interactive StartSlice requires the actual target protocol371 before sending; missing/noninteger/legacy advertisement fails with an upgrade action. Noninteractive shape remains unchanged. The kernel owns export review.",
    "anchors": [
      [
        "client_projection",
        "assertInteractiveSliceStartSupported"
      ],
      [
        "client_projection",
        "startSliceRequest"
      ]
    ],
    "blob": "788f010e49dd5c50d40e1a4bb8fb5d95572a34ab",
    "openFindings": [
      "MP-08/MP-10/MP-11: independent exact-head source review and signed/fresh-machine acceptance remain open."
    ]
  },
  {
    "id": "parity3-interactive-slice-create-admission",
    "sourceCommit": "d0638173d10e1fa6040741d72da455d9847b4322",
    "path": "apps/web/src/terminal/slice-launch-helpers.ts",
    "ranges": [
      [
        53,
        88
      ],
      [
        88,
        119
      ]
    ],
    "classification": "shared_protocol_feature_admission",
    "rationale": "MP-08/MP-11: Interactive create checks the connected client target protocol before CreateSlice; every StartSlice goes through the same request admission. Helpers retain common scope checks and forward kernel review without client approval authority.",
    "anchors": [
      [
        "client_projection",
        "createAndStartSlice"
      ],
      [
        "client_projection",
        "ensureSliceStarted"
      ]
    ],
    "blob": "9529266788619d744c966071a369ed2617c2810d",
    "openFindings": [
      "MP-08/MP-10/MP-11: independent exact-head source review and signed/fresh-machine acceptance remain open."
    ]
  },
  {
    "id": "impld-path1-role-service-policy",
    "sourceCommit": "6263ab3e124f0fbce71b87b9a3bb9e51bcf8c25f",
    "path": "deploy/managed-kernel/path1-service-policy.mjs",
    "blob": "e3c5ccc236de6597f79661d7a2c2d5ea81149bcc",
    "ranges": [
      [
        1,
        110
      ]
    ],
    "classification": "signed_release_role_policy_guard",
    "rationale": "MP-01/MP-04/MP-07/MP-11: one parsed Unit/Service policy verifies both signed Path-1 roles, direct ExecStart, user HOME/state/topology/PATH, private kernel broker restart/socket and provider-inherited deny list. Worker StateDirectory allocates only the intentional chariox control directory. Twelve independently signed worker mutations failed before correction; role units remain ordinary provider launch. Only signed deployment is an exception here.",
    "anchors": [
      [
        "release_activation",
        "verifyPath1ServicePolicy"
      ],
      [
        "release_activation",
        "parseUnitSections"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  },
  {
    "id": "impld-repository-component-safety",
    "sourceCommit": "6263ab3e124f0fbce71b87b9a3bb9e51bcf8c25f",
    "path": "apps/kernel/src/managed_context/development/export.rs",
    "blob": "2345bc99727316145449e3e1c00443ba4e655c71",
    "ranges": [
      [
        481,
        533
      ]
    ],
    "classification": "shared_repository_component_safety",
    "rationale": "MP-05/MP-06/MP-11: ordinary single-component basename safety now governs managed export too. Safe generic names and .chariox names are exportable; a managed copy still preserves source basename and rejects selected-name collisions. There is no name-only runtime blacklist and no new exception. Actual destination controls are checked during import.",
    "anchors": [
      [
        "protected_path_filter",
        "target_directory_for_export"
      ],
      [
        "protected_path_filter",
        "validate_repository_basename"
      ],
      [
        "protected_path_filter",
        "unique_managed_target_directory"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  },
  {
    "id": "impld-repository-final-destination",
    "sourceCommit": "6263ab3e124f0fbce71b87b9a3bb9e51bcf8c25f",
    "path": "apps/kernel/src/managed_context/development/import.rs",
    "blob": "3ae084bed7404564814783a275c42d6eead24e67",
    "ranges": [
      [
        507,
        519
      ],
      [
        747,
        754
      ],
      [
        1081,
        1124
      ],
      [
        1320,
        1360
      ],
      [
        1573,
        1618
      ]
    ],
    "classification": "actual_repository_destination_control_and_ownership",
    "rationale": "MP-03/MP-05/MP-06/MP-11: publication import/receipt/rollback uses the shared component validator. Canonical trusted roots, protected actual final destinations, no-clobber occupancy/symlink checks and inode-owned rollback remain. Receipt replay accepts safe names through product recovery. The protected .chariox destination remains rejected without treating all same-named repositories as state.",
    "anchors": [
      [
        "protected_path_filter",
        "preflight_managed_repository_path"
      ],
      [
        "protected_path_filter",
        "publication_materialization_root"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  },
  {
    "id": "impld-auto-stop-outage-policy",
    "sourceCommit": "8fac1a2c8bc5c208ce4d71a43ff978b8b070a451",
    "path": "apps/api/src/managed-environments/auto-stop-outage-policy.ts",
    "blob": "7918b0b8be76ea8db692e29ef74c789b432e3d2a",
    "classification": "mandatory_shutdown_bounded_acknowledgement_outage",
    "rationale": "MP-09/MP-11: the authorized 60-minute allowance is bound to durable challenge creation/deadline/busy ACK; process restart cannot renew it. The serializable transaction checks enabled policy/minimum runtime and current account/Machine/generation/kernel grant, signed activity and Machine/relay heartbeat freshness. Silence grants no kernel admission fence. Mandatory shutdown is the only exception used.",
    "anchors": [
      [
        "automatic_shutdown_selector",
        "canStopAfterAcknowledgementOutage"
      ],
      [
        "automatic_shutdown_selector",
        "autoStopAcknowledgementOutageMs"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  },
  {
    "id": "impld-auto-stop-operation",
    "sourceCommit": "8fac1a2c8bc5c208ce4d71a43ff978b8b070a451",
    "path": "apps/api/src/managed-environments/auto-stop-operation.ts",
    "blob": "25bd7beb2f3b62329da8e94006c5625cfe0171af",
    "classification": "mandatory_shutdown_common_stop_operation",
    "rationale": "MP-09/MP-11: acknowledged and outage paths queue the same normal non-destructive STOP with the durable operation/idempotency/generation binding, desired-state CAS and normal provider retry. Durable audit metadata distinguishes lack of ACK; successful STOP projects the reason through existing summary error fields, including after retry. No DELETE, disk/context reset or protocol shape is added. This is mandatory shutdown only.",
    "anchors": [
      [
        "automatic_shutdown_selector",
        "requestAutoStop"
      ],
      [
        "automatic_shutdown_selector",
        "autoStopCompletionReason"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  },
  {
    "id": "impld-auto-stop-reservation",
    "sourceCommit": "8fac1a2c8bc5c208ce4d71a43ff978b8b070a451",
    "path": "apps/api/src/managed-environments/auto-stop-quiescence-service.ts",
    "blob": "0049a61ffc002f5525c9bdd6ada78ac21888d418",
    "ranges": [
      [
        54,
        347
      ],
      [
        448,
        525
      ],
      [
        584,
        605
      ]
    ],
    "classification": "mandatory_shutdown_reservation_and_bounded_outage_dispatch",
    "rationale": "MP-09/MP-11: the ordinary durable quiescence reservation keeps account/activity/deadline/revision checks and unsettled operation protection. The five-minute warning now describes the bounded stale-heartbeat STOP; after the outage cutoff the same transaction queues a normal STOP, with no inferred fence acknowledgement. ACK and fallback races produce one STOP in the disposable database. Only mandatory automatic shutdown is an exception.",
    "anchors": [
      [
        "automatic_shutdown_selector",
        "createManagedEnvironmentAutoStopQuiescenceService"
      ],
      [
        "automatic_shutdown_selector",
        "reservationStillDue"
      ],
      [
        "automatic_shutdown_selector",
        "hasUnsettledOperation"
      ]
    ],
    "openFindings": [
      "MP-10/MP-11: implementation observation only; independent changed-source review, signed aggregate/fresh-machine comparison, live provider/user-state proof and cleanup remain open. Frozen removal dispositions remain tied to their original sources."
    ]
  }
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
      // limited to a rule's named keys and ranges, including compact JSON; it grants no semantic approval.
      const jsonProperty = file.path.endsWith(".json")
        ? new RegExp("(?:^|[,{])\\s*" + JSON.stringify(symbol) + "\\s*:") : null;
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
    const applies = [CLOUD, CLOUD_CURRENT, CLOUD_PARITY3, CLOUD_IMPLD].includes(rule.sourceCommit) ? cloud : oss;
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
