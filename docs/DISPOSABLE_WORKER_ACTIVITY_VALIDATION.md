# Disposable worker activity reporting

## Integration status

The runtime integration branch starts from account-recovery head `3b76570ba7`
and ports bootstrap/reporter changes from PRs #317/#318. It uses the existing
`RemoteLeaseWorker` role and `remote_lease_capacity`, not a second capacity-based
role. Selected-home admission additionally pins kernel, realm, user, and sender
key before the existing lease-ownership checks. The account-profile and remote
binding implementations are unchanged from that newer base.

At combined source revision `c3d6f629c98377b0706873145fa82b20e803c795`, kernel
and test targets type-checked and 62 focused test executions passed, covering
60 distinct tests. These comprise 13 reporter, two bootstrap, four activity-state,
one selected-home identity, five role configuration, three lease-authority,
eight remote provider-account, and 26 replica-filter executions. Two account
tests occur in both final groups. All groups ran serially using the same compiled
test binary. No GitHub CI ran. These results do not constitute deployment approval.

Linux release packaging and build-context suites passed together, 17/17, on the
temporary Hetzner builder with Node 22.22.1. The tested export was from
`2c9e550723b2f644c501775b02e065f73dcba0b6`; its temporary Git fixture tree was
verified equal to `d4f354ca90256293c724282b7859b2d73bc98f30`. The first archive-only
attempt failed the build-context test because Git metadata was absent; the
complete rerun followed fixture correction. These tests use synthetic binaries
and test signing keys, not a new compiled or deployed Linux release.
The older disabled bootstrap contract and the allocation exchange/confirm path
still need consolidation before this combined worker implementation is deployed.

The disposable worker supervisor passes its enrollment receipt to the ordinary
kernel activity reporter. The reporter validates the worker Machine, kernel,
relay public key, Cloud profile, capacity, and home caller before reporting.
It does not transfer or refresh provider credentials.

The child starts before enrollment confirmation. An exchanged receipt therefore
enables reporting; Cloud rejects reports until enrollment is confirmed, and the
existing reporter retry loop handles that interval. Restart cursor recovery,
lost acknowledgements, and activity transitions use the same implementation as
independent managed kernels.

Worker reports use `/v1/disposable-workers/activity` with `allocationId` instead
of `environmentId`. The signed fields are accountId, allocationId, kernelId,
machineId, runningAgentCount, and sequence, sorted in that order. The Machine
credential is the HMAC-SHA256 key. Independent managed kernels retain their
existing endpoint and signature contract.

Paired Cloud implementation: chariox-cloud PR #92, head
`f7bc768c4117a1e42c112de3f2408eca145bbf65`.

## Local validation

At source revision `c573e5ccb0bb4840d05a64a21ac9bbf431ae8bf4`, the kernel and
test targets type-checked. All 13 reporter tests, two worker bootstrap tests,
and four shared activity-state tests passed. The latter cover queued prompts,
unresolved interactions, and active-turn settlement. Compilation used one job,
no incremental cache, and no debug information. GitHub CI was not run.

## Validation still required before deployment acceptance

The first remote upgrade-suite run exposed two integration defects: requiring
the new worker service in old signed manifests blocked existing installations,
and staging omitted that service from new releases. The upgrade fixture now
exercises a legacy installed image and worker-capable target. Verification
accepts both exact signed artifact sets, rejects an undeclared worker service,
and continues checking the declared service hash. Upgrade staging copies the
service when present. The 39-test upgrade/rollback suite passed on the original
managed machine using temporary roots and a fake service manager; after the
additional undeclared-service guard, three focused cases passed. No live upgrade
or restart occurred. All 17 packaging/build-context tests also passed with the
final verifier changes. Packaging fixtures still use synthetic binaries and
the committed source base, so these results do not attest a new release image.

- Exercise bootstrap confirmation delay, restart cursor recovery, and
  busy-to-idle reporting against the paired Cloud service.
- Verify queued prompts and unresolved permissions prevent idle release.
- Verify keep-running, fresh work cancelling a deadline, and disconnected clients.
- Verify VM and billable-resource deletion after release without changing ordinary
  managed-machine stop/start persistence.

Local type-checking is not proof of those live lifecycle requirements. No local
daemon or relay serialized shape is changed by this HTTP reporter connection.
