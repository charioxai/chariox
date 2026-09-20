import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const managedServiceUrl = new URL(
  "../deploy/managed-kernel/chariox-managed-bootstrap.service",
  import.meta.url,
)
const disposableWorkerServiceUrl = new URL(
  "../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service",
  import.meta.url,
)
const imagePreparationUrl = new URL(
  "../deploy/managed-kernel/prepare-hetzner-image.sh",
  import.meta.url,
)
const managedSupervisorUrl = new URL(
  "../apps/kernel/src/managed_bootstrap/supervisor.rs",
  import.meta.url,
)
const disposableWorkerUrl = new URL(
  "../apps/kernel/src/managed_bootstrap/worker.rs",
  import.meta.url,
)
const dockerSliceRuntimeUrl = new URL(
  "../apps/kernel/slice-linux-docker/docker/start-runtime.sh",
  import.meta.url,
)

const managedIsolationMarker = "CHARIOX_MANAGED_PROVIDER_ISOLATION"
const inheritedProviderRestrictions = [
  "NoNewPrivileges",
  "PrivateTmp",
  "ProtectSystem",
  "ProtectHome",
  "ProtectKernelTunables",
  "ProtectKernelModules",
  "ProtectControlGroups",
  "RestrictSUIDSGID",
  "RestrictAddressFamilies",
  "ReadWritePaths",
  "UMask",
]

test("Path-1 providers inherit an ordinary worker process environment", async () => {
  const [managedService, disposableWorkerService, supervisor, worker, imagePreparation] =
    await Promise.all([
      readFile(managedServiceUrl, "utf8"),
      readFile(disposableWorkerServiceUrl, "utf8"),
      readFile(managedSupervisorUrl, "utf8"),
      readFile(disposableWorkerUrl, "utf8"),
      readFile(imagePreparationUrl, "utf8"),
    ])

  assert.match(disposableWorkerService, /^Environment=HOME=\/home\/chariox$/m)
  assert.match(
    disposableWorkerService,
    /^Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox$/m,
  )
  assert.match(
    disposableWorkerService,
    /^Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1$/m,
  )
  assert.doesNotMatch(
    disposableWorkerService,
    new RegExp(`^Environment=${managedIsolationMarker}=`, "m"),
    "the disposable Path-1 worker must not activate managed provider isolation",
  )
  for (const restriction of inheritedProviderRestrictions) {
    assert.doesNotMatch(
      disposableWorkerService,
      new RegExp(`^${restriction}=`),
      `the disposable Path-1 worker must not impose ${restriction} on provider descendants`,
    )
  }

  const isolationRemoval = new RegExp(
    String.raw`\.env_remove\(\s*"${managedIsolationMarker}"\s*\)`,
  )
  assert.match(worker, isolationRemoval, "disposable worker must scrub inherited isolation")
  assert.match(
    imagePreparation,
    /if \[ "\$managed_provider_topology" = shared_host \]; then[\s\S]*verify-provider-runtime-bind\.sh/,
    "image preparation must gate the Bubblewrap probe to shared-host mode",
  )
})

test("legacy shared-host providers retain their explicit inner boundary", async () => {
  const [managedService, supervisor] = await Promise.all([
    readFile(managedServiceUrl, "utf8"),
    readFile(managedSupervisorUrl, "utf8"),
  ])

  assert.match(managedService, /^Environment=HOME=\/home\/chariox$/m)
  assert.match(managedService, /^Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox$/m)
  assert.match(
    managedService,
    /^Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host$/m,
  )
  assert.match(
    managedService,
    new RegExp(`^Environment=${managedIsolationMarker}=1$`, "m"),
  )
  for (const restriction of inheritedProviderRestrictions) {
    assert.match(
      managedService,
      new RegExp(`^${restriction}=`, "m"),
      `the shared-host service must retain ${restriction}`,
    )
  }
  assert.match(
    supervisor,
    new RegExp(String.raw`\.env\(\s*"${managedIsolationMarker}"\s*,\s*"1"\s*\)`),
    "the shared-host supervisor must retain explicit managed isolation",
  )
})

test("Docker slices retain their explicit inner provider boundary", async () => {
  const dockerSliceRuntime = await readFile(dockerSliceRuntimeUrl, "utf8")

  assert.match(
    dockerSliceRuntime,
    /^\s*CHARIOX_MANAGED_PROVIDER_ISOLATION=1 \\$/m,
    "the Docker-slice topology must retain its independent inner boundary",
  )
  assert.match(
    dockerSliceRuntime,
    /^\s*CHARIOX_MANAGED_PROVIDER_BWRAP="\/usr\/local\/libexec\/chariox\/managed-provider-bwrap" \\$/m,
  )
})
