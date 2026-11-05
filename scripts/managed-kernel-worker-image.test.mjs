import assert from "node:assert/strict"
import { once } from "node:events"
import { access, chmod, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn, spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const rootfulSocketHelperUrl = new URL(
  "../deploy/managed-kernel/remove-stale-rootful-docker-socket.sh",
  import.meta.url,
)
const providerRuntimeBindUrl = new URL(
  "../deploy/managed-kernel/verify-provider-runtime-bind.sh",
  import.meta.url,
)
const providerLaunchProbeUrl = new URL(
  "../deploy/managed-kernel/probe-provider-launch-ab.sh",
  import.meta.url,
)
const publicationAccessDrillUrl = new URL(
  "../deploy/managed-kernel/verify-slice-publication-access.sh",
  import.meta.url,
)
const versionsUrl = new URL("../deploy/managed-kernel/provider-versions.env", import.meta.url)
const publicationDockerfileUrl = new URL("../docker/publication/Dockerfile", import.meta.url)
const sliceDockerfileUrl = new URL("../apps/kernel/slice-linux-docker/docker/Dockerfile", import.meta.url)
const sliceToolchainPackageUrl = new URL("../apps/kernel/slice-linux-docker/toolchain/package.json", import.meta.url)
const sliceToolchainLockUrl = new URL("../apps/kernel/slice-linux-docker/toolchain/package-lock.json", import.meta.url)
const runbookUrl = new URL("../docs/MANAGED_REMOTE_KERNEL_IMAGE.md", import.meta.url)
const bootstrapEntrypointUrl = new URL("../apps/kernel/src/bin/chariox-managed-bootstrap.rs", import.meta.url)
const managedBootstrapStateUrl = new URL("../apps/kernel/src/managed_bootstrap/state.rs", import.meta.url)
const imageReleaseVerifierUrl = new URL("../deploy/managed-kernel/verify-image-release.mjs", import.meta.url)
const managedServiceUrl = new URL("../deploy/managed-kernel/chariox-managed-bootstrap.service", import.meta.url)
const path1ManagedServiceUrl = new URL("../deploy/managed-kernel/chariox-path1-managed-bootstrap.service", import.meta.url)
const workerServiceUrl = new URL("../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service", import.meta.url)
const workerKernelSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/worker.rs", import.meta.url)
const workerSupervisorSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/supervisor.rs", import.meta.url)
const rootlessServiceUrl = new URL("../deploy/managed-kernel/chariox-rootless-docker.service", import.meta.url)
const brokerServiceUrl = new URL("../deploy/managed-kernel/chariox-slice-broker.service", import.meta.url)
const installerUrl = new URL("../deploy/managed-kernel/install-image.sh", import.meta.url)
const publicationAccessUrl = new URL("../apps/kernel/slice-linux-docker/managed-publication-access.sh", import.meta.url)
const sliceDevelopmentRuntimeStateUrl = new URL(
  "../apps/kernel/src/runtime/state/slice_development_runtime_state.rs",
  import.meta.url,
)
const sliceEmptyDevelopmentUrl = new URL(
  "../apps/kernel/src/runtime/state/slice_empty_development.rs",
  import.meta.url,
)
const managedBrokerUrl = new URL("../apps/kernel/slice-linux-docker/managed-docker-broker.mjs", import.meta.url)
const rootlessNamespaceUrl = new URL(
  "../apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh",
  import.meta.url,
)
const sliceProvisionerUrl = new URL(
  "../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  import.meta.url,
)

test("managed image installer rejects omitted topology before staging", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "chariox-installer-topology-"))
  try {
    const fakeId = join(scratch, "id")
    await writeFile(fakeId, "#!/bin/sh\nprintf '0\\n'\n")
    await chmod(fakeId, 0o700)

    const result = spawnSync(
      fileURLToPath(installerUrl),
      ["/nonexistent/managed-rootfs", "sha256:invalid", "/nonexistent/trusted-key"],
      {
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: `${scratch}:${process.env.PATH ?? "/usr/bin:/bin"}`,
          TMPDIR: scratch,
        },
      },
    )
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /usage: install-image\.sh .*<path1\|shared_host>/)
    assert.deepEqual(await readdir(scratch), ["id"])
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})






test("rootful Docker socket cleanup fails closed and removes only stale sockets", async () => {
  const helperPath = fileURLToPath(rootfulSocketHelperUrl)
  const lsofResult = spawnSync("sh", ["-c", "command -v lsof"], { encoding: "utf8" })
  assert.equal(lsofResult.status, 0, lsofResult.stderr)
  const lsofPath = lsofResult.stdout.trim()
  assert.ok(lsofPath.startsWith("/"), "lsof must resolve to an absolute path")

  const fixtureRoot = await mkdtemp(join(tmpdir(), "chariox-rootful-docker-socket-"))
  try {
    const absentSocket = join(fixtureRoot, "absent.sock")
    let result = spawnSync(helperPath, [absentSocket, "inactive", "inactive", lsofPath], {
      encoding: "utf8",
    })
    assert.equal(result.status, 0, result.stderr)

    const staleSocket = join(fixtureRoot, "stale.sock")
    result = spawnSync(
      "python3",
      ["-c", "import socket,sys; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.close()", staleSocket],
      { encoding: "utf8" },
    )
    assert.equal(result.status, 0, result.stderr)
    result = spawnSync(helperPath, [staleSocket, "inactive", "inactive", lsofPath], {
      encoding: "utf8",
    })
    assert.equal(result.status, 0, result.stderr)

    const liveSocket = join(fixtureRoot, "live.sock")
    const server = await new Promise((resolve, reject) => {
      const child = spawn(
        "python3",
        [
          "-u",
          "-c",
          "import socket,sys,time; s=socket.socket(socket.AF_UNIX); s.bind(sys.argv[1]); s.listen(1); print('ready'); time.sleep(30)",
          liveSocket,
        ],
        { stdio: ["ignore", "pipe", "pipe"] },
      )
      child.once("error", reject)
      child.stdout.once("data", () => resolve(child))
    })
    try {
      result = spawnSync(helperPath, [liveSocket, "inactive", "inactive", lsofPath], {
        encoding: "utf8",
      })
      assert.notEqual(result.status, 0)
      assert.match(result.stderr, /still owned by process/)
    } finally {
      server.kill("SIGTERM")
      await new Promise((resolve) => server.once("exit", resolve))
    }

    const unexpectedFile = join(fixtureRoot, "docker.sock")
    await writeFile(unexpectedFile, "not a socket\n")
    result = spawnSync(helperPath, [unexpectedFile, "inactive", "inactive", lsofPath], {
      encoding: "utf8",
    })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /is not a trusted Unix socket/)

    result = spawnSync(helperPath, [absentSocket, "active", "inactive", lsofPath], {
      encoding: "utf8",
    })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /docker\.service remained active/)
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true })
  }
})


test("managed image and publication runtimes pin the same provider releases", async () => {
  const versions = Object.fromEntries(
    (await readFile(versionsUrl, "utf8"))
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  )
  const dockerfile = await readFile(publicationDockerfileUrl, "utf8")
  assert.deepEqual(versions, {
    CHARIOX_CODEX_VERSION: "0.159.3",
    CHARIOX_OPENCODE_VERSION: "1.19.0",
    CHARIOX_CLAUDE_VERSION: "2.1.212",
  })
  for (const [name, version] of Object.entries(versions)) {
    assert.match(dockerfile, new RegExp(`ARG ${name}=${version.replaceAll(".", "\\.")}`))
  }
})

test("managed slice image locks every network and compiler input", async () => {
  const dockerfile = await readFile(sliceDockerfileUrl, "utf8")
  const toolchainPackage = JSON.parse(await readFile(sliceToolchainPackageUrl, "utf8"))
  const toolchainLock = JSON.parse(await readFile(sliceToolchainLockUrl, "utf8"))

  const fromStages = [...dockerfile.matchAll(/^FROM\s+(\S+)(?:\s+AS\s+(\S+))?\s*$/gim)]
  assert.ok(fromStages.length > 0, "the slice image must declare base stages")
  const scratchStages = fromStages.filter(([, image]) => image.toLowerCase() === "scratch")
  assert.deepEqual(
    scratchStages.map(([, image, stage]) => [image, stage]),
    [["scratch", "managed-release-artifacts"]],
    "only the exact runtime-artifact export stage may use scratch",
  )
  const scratchStageStart = dockerfile.indexOf("FROM scratch AS managed-release-artifacts")
  const scratchStageEnd = dockerfile.indexOf("\nFROM ", scratchStageStart + 1)
  assert.ok(scratchStageStart >= 0 && scratchStageEnd > scratchStageStart)
  assert.match(
    dockerfile,
    /cargo build --locked --release --package chariox-app-wasm-worker --bin chariox-app-wasm-worker/,
    "the builder must compile the WASM App worker that the managed kernel opens beside itself",
  )
  assert.deepEqual(
    dockerfile.slice(scratchStageStart, scratchStageEnd).trim().split("\n"),
    [
      "FROM scratch AS managed-release-artifacts",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-kernel /chariox-kernel",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-managed-bootstrap /chariox-managed-bootstrap",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-relay /chariox-relay",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-app-package /chariox-app-package",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-app-wasm-worker /chariox-app-wasm-worker",
    ],
    "scratch may export only the five managed runtime binaries",
  )
  for (const [, image] of fromStages) {
    if (image.toLowerCase() === "scratch") continue
    assert.match(image, /@sha256:[a-f0-9]{64}$/, `${image} must be digest-pinned`)
  }
  assert.match(dockerfile, /snapshot\.debian\.org\/archive\/debian\/20260701T000000Z/)
  assert.match(dockerfile, /COPY Cargo\.toml Cargo\.lock \.\//)
  assert.match(dockerfile, /cargo build --locked --release/)
  assert.match(dockerfile, /groupadd --gid 1001 slice/)
  assert.match(dockerfile, /useradd --uid 1001 --gid 1001/)
  assert.match(dockerfile, /npm ci --omit=dev/)
  assert.match(dockerfile, /npm_config_cache=\/tmp\/chariox-npm-cache/)
  assert.match(dockerfile, /node_modules\/\.bin\/pnpm --version/)
  assert.match(dockerfile, /rm -rf \/tmp\/chariox-npm-cache \/root\/\.npm/)
  assert.match(
    dockerfile,
    /^# syntax=docker\/dockerfile:1@sha256:[a-f0-9]{64}$/m,
  )
  assert.doesNotMatch(dockerfile, /npm install|rustup\.rs|deb\.nodesource\.com/)
  assert.deepEqual(toolchainPackage.dependencies, {
    "@anthropic-ai/claude-code": "2.1.212",
    "@openai/codex": "0.159.3",
    "opencode-ai": "1.19.0",
    pnpm: "11.22.0",
    ws: "8.21.3",
  })
  assert.equal(toolchainLock.lockfileVersion, 3)
  for (const [path, entry] of Object.entries(toolchainLock.packages)) {
    if (!path || entry.link) continue
    assert.match(entry.resolved ?? "", /^https:\/\/registry\.npmjs\.org\//, `${path} needs a registry artifact`)
    assert.match(entry.integrity ?? "", /^sha512-/, `${path} needs SHA-512 integrity`)
  }
})

test("managed slices use builder-attested runtime binaries instead of compiling on the host", async () => {
  const dockerfile = await readFile(sliceDockerfileUrl, "utf8")
  const provisioner = await readFile(sliceProvisionerUrl, "utf8")

  assert.match(dockerfile, /ARG CHARIOX_PREBUILT_RUNTIME=0/)
  assert.match(dockerfile, /test -x \/opt\/chariox-prebuilt\/chariox-kernel/)
  assert.match(dockerfile, /test -x \/opt\/chariox-prebuilt\/chariox-relay/)
  assert.match(provisioner, /prebuilt\/\.managed-release/)
  assert.match(provisioner, /--build-arg "CHARIOX_PREBUILT_RUNTIME=1"/)
})

test("managed service delegates cgroup controllers without the removed App storage helper", async () => {
  // WASM Apps run inside the kernel's own worker, so no root helper prepares an App domain.
  const managed = await readFile(managedServiceUrl, "utf8")
  assert.match(managed, /^Delegate=cpu memory pids$/m)
  assert.match(managed, /^DelegateSubgroup=supervisor$/m)
  assert.doesNotMatch(managed, /chariox-app-storage|--prepare-managed-domain/)
})

test("managed Docker authority and publication access remain narrowly separated", async () => {
  const bootstrapEntrypoint = await readFile(bootstrapEntrypointUrl, "utf8")
  const managed = await readFile(managedServiceUrl, "utf8")
  const worker = await readFile(workerServiceUrl, "utf8")
  const workerKernel = await readFile(workerKernelSourceUrl, "utf8")
  const workerSupervisor = await readFile(workerSupervisorSourceUrl, "utf8")
  const providerLaunchProbe = await readFile(providerLaunchProbeUrl, "utf8")
  const rootless = await readFile(rootlessServiceUrl, "utf8")
  const broker = await readFile(brokerServiceUrl, "utf8")
  const installer = await readFile(installerUrl, "utf8")
  const accessHelper = await readFile(publicationAccessUrl, "utf8")
  const managedBroker = await readFile(managedBrokerUrl, "utf8")
  const rootlessNamespace = await readFile(rootlessNamespaceUrl, "utf8")

  assert.match(bootstrapEntrypoint, /\(Some\("--disposable-worker"\), 1\) => true/)
  assert.match(
    bootstrapEntrypoint,
    /if disposable_worker \{[\s\S]*worker::run_from_env\(\)/,
  )
  assert.match(managed, /Wants=network-online\.target chariox-rootless-docker\.service/)
  assert.doesNotMatch(managed, /(?:Wants|After)=.*chariox-slice-broker/)
  assert.match(managed, /ExecStartPre=-\+\/usr\/bin\/systemctl restart chariox-slice-broker\.service/)
  assert.match(managed, /Environment=HOME=\/home\/chariox/)
  assert.match(managed, /Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox/)
  assert.match(managed, /CHARIOX_CAPABILITY_ISOLATION_ROOT=\/home\/chariox\/\.chariox\/managed-context\/kernel/)
  assert.match(managed, /Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host/)
  assert.match(managed, /Environment=CHARIOX_MANAGED_PROVIDER_ISOLATION=1/)
  assert.match(managed, /CHARIOX_MANAGED_VAULT_PATH=\/home\/chariox\/\.chariox\/vault\/vault\.json/)
  assert.match(managed, /CHARIOX_MANAGED_BOOTSTRAP_RECEIPT=\/var\/lib\/chariox\/managed\/bootstrap-receipt\.json/)
  assert.match(managed, /ProtectHome=read-only/)
  assert.match(managed, /ReadWritePaths=\/home\/chariox \/var\/lib\/chariox \/var\/lib\/chariox-slice-share/)
  assert.match(managed, /Restart=always/)
  assert.match(managed, /RestartSteps=8/)
  assert.match(managed, /RestartMaxDelaySec=5min/)
  assert.match(managed, /^ProtectKernelTunables=false$/m)
  for (const directive of [
    "NoNewPrivileges=true",
    "PrivateTmp=true",
    "ProtectSystem=strict",
    "ProtectHome=read-only",
    "ProtectKernelModules=true",
    "ProtectControlGroups=false",
    "RestrictSUIDSGID=true",
    "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK",
    "UMask=0007",
  ]) {
    assert.match(managed, new RegExp(`^${directive}$`, "m"))
  }
  assert.match(worker, /Environment=HOME=\/home\/chariox/)
  assert.match(worker, /Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox/)
  assert.match(worker, /Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1/)
  assert.match(worker, /Environment=CHARIOX_DISPOSABLE_WORKER_RECEIPT=\/var\/lib\/chariox\/disposable-worker\/bootstrap-receipt\.json/)
  assert.doesNotMatch(worker, /CHARIOX_MANAGED_PROVIDER_HOME/)
  assert.match(worker, /Environment=CHARIOX_MANAGED_VAULT_PATH=\/home\/chariox\/\.chariox\/vault\/vault\.json/)
  // The supervisor needs this endpoint to claim the one-shot broker. Its
  // child kernel receives only the scoped lease FD, never the socket path.
  assert.match(worker, /^Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=\/var\/lib\/chariox-slice-share\/\.broker-private\/control\/control\.sock$/m)
  assert.match(
    workerKernel,
    /super::supervisor::spawn_with_broker_lease\(&mut command,\s*topology\)/,
    "worker kernel launch must delegate to the supervisor broker boundary",
  )
  const brokerSpawnStart = workerSupervisor.indexOf("pub(super) fn spawn_with_broker_lease(")
  const brokerSpawnEnd = workerSupervisor.indexOf("\n#[cfg(all(test, unix))]", brokerSpawnStart)
  assert.ok(brokerSpawnStart >= 0 && brokerSpawnEnd > brokerSpawnStart)
  const brokerSpawn = workerSupervisor.slice(brokerSpawnStart, brokerSpawnEnd)
  const path1BoundaryStart = brokerSpawn.indexOf("if topology == ManagedProviderTopology::Path1 {")
  const path1BoundaryEnd = brokerSpawn.indexOf("\n\n    #[cfg(unix)]", path1BoundaryStart)
  assert.ok(path1BoundaryStart >= 0 && path1BoundaryEnd > path1BoundaryStart)
  const path1Boundary = brokerSpawn.slice(path1BoundaryStart, path1BoundaryEnd)
  assert.match(
    path1Boundary,
    /for name in PATH1_SHARED_HOST_SELECTOR_ENVS\s*\{\s*command\.env_remove\(name\);\s*\}/,
  )
  assert.match(
    path1Boundary,
    /for name in PATH1_KERNEL_SLICE_BROKER_ENVS\s*\{\s*command\.env_remove\(name\);\s*\}/,
  )
  assert.match(
    path1Boundary,
    /path1_managed_slice_root_from_broker_socket\(\)\s*\{\s*command\.env\("CHARIOX_SLICE_ROOT", slice_root\);/,
  )
  assert.match(
    workerSupervisor,
    /fn path1_managed_slice_root_from_broker_socket\(\)\s*-> Option<std::path::PathBuf>\s*\{\s*broker_share_root_from_socket\(\)\.map\(\|share_root\| share_root\.join\("slices"\)\)\s*\}/,
  )
  assert.match(
    brokerSpawn,
    /command\s*\.env_remove\(BROKER_SOCKET_ENV\)\s*\.env_remove\(BROKER_REQUIRED_ENV\)\s*\.env\(BROKER_FD_ENV, fd\.to_string\(\)\)/,
    "the supervisor must pass an active lease by FD and omit the socket path",
  )
  assert.match(
    brokerSpawn,
    /command\s*\.env_remove\(BROKER_SOCKET_ENV\)\s*\.env_remove\(BROKER_FD_ENV\)\s*\.env\(BROKER_REQUIRED_ENV, "1"\)/,
    "a missing lease must fail closed with the required-broker marker",
  )
  for (const sharedHostSelector of [
    "CHARIOX_CAPABILITY_ISOLATION_ROOT=",
    "CHARIOX_MANAGED_PROVIDER_ISOLATION=",
    "CHARIOX_MANAGED_PROVIDER_BWRAP=",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT=",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT=",
    "CHARIOX_SLICE_ROOT=",
    "CHARIOX_SLICE_DOCKER_BROKER_FD=",
    "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED=",
  ]) {
    assert.doesNotMatch(worker, new RegExp(`^Environment=${sharedHostSelector}`, "m"))
  }
  assert.match(worker, /Restart=always/)
  assert.match(worker, /RestartSteps=8/)
  assert.match(worker, /RestartMaxDelaySec=5min/)
  assert.match(worker, /Path 1 is one disposable VM per worker/)
  assert.doesNotMatch(worker, /CHARIOX_MANAGED_PROVIDER_ISOLATION=/)
  assert.doesNotMatch(worker, /CHARIOX_CAPABILITY_ISOLATION_ROOT=/)
  assert.doesNotMatch(worker, /bwrap/i)
  for (const directive of [
    "NoNewPrivileges=",
    "PrivateTmp=",
    "ProtectSystem=",
    "ProtectHome=",
    "ProtectKernelTunables=",
    "ProtectKernelModules=",
    "ProtectControlGroups=",
    "RestrictSUIDSGID=",
    "RestrictAddressFamilies=",
    "ReadWritePaths=",
    "UMask=",
  ]) {
    assert.doesNotMatch(worker, new RegExp(`^${directive}`, "m"))
  }
  assert.doesNotMatch(rootless, /SupplementaryGroups=chariox-slice/)
  assert.match(rootless, /Environment=PATH=\/usr\/local\/bin:\/usr\/bin:\/bin:\/usr\/sbin:\/sbin/)
  assert.match(rootless, /^ProtectKernelTunables=false$/m)
  assert.match(rootless, /^RestrictSUIDSGID=true$/m)
  assert.match(rootless, /ExecStart=.*managed-rootless-service\.sh start/)
  assert.match(rootless, /ExecStopPost=.*managed-rootless-service\.sh stop/)
  assert.match(rootless, /ExecStartPost=.*managed-rootless-service\.sh ready/)
  const engine = await readFile(new URL("../apps/kernel/slice-linux-docker/chariox-rootless-engine.service", import.meta.url), "utf8")
  assert.match(engine, /--exec-opt native\.cgroupdriver=systemd/)
  assert.match(engine, /^Delegate=cpu cpuset io memory pids$/m)
  assert.doesNotMatch(engine, /^User=/m)
  assert.match(installer, /user@\$docker_uid\.service\.d/)
  assert.match(installer, /loginctl enable-linger chariox-docker/)
  assert.match(rootless, /ReadWritePaths=.*\/var\/lib\/chariox-slice-share\/\.broker-private/)
  assert.match(rootless, /ReadWritePaths=.*\/var\/lib\/chariox-slice-share\/slices\/development/)
  assert.doesNotMatch(rootless, /ReadWritePaths=.*\/var\/lib\/chariox(?:\/home)?(?:\s|$)/)
  assert.match(broker, /^Restart=no$/m)
  // The kernel delegates its App subtree; the Docker broker has no such authority.
  assert.match(broker, /^ProtectControlGroups=true$/m)
  assert.match(broker, /^Group=chariox-docker$/m)
  assert.doesNotMatch(broker, /^SupplementaryGroups=/m)
  assert.match(broker, /enter-rootless-docker-namespace\.sh \/usr\/bin\/node/)
  assert.match(broker, /CHARIOX_SLICE_DOCKER_BROKER_SOCKET=\/var\/lib\/chariox-slice-share\/\.broker-private\/control\/control\.sock/)
  assert.match(rootlessNamespace, /nsenter --target "\$child_pid" --user --mount/)
  assert.match(rootlessNamespace, /nsenter --target "\$child_pid" --user --mount --net -- "\$@"/)
  assert.match(rootlessNamespace, /\[ -L "\/proc\/\$child_pid\/ns\/net" \]/)
  assert.match(providerLaunchProbe, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
  assert.doesNotMatch(providerLaunchProbe, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1/)
  assert.match(providerLaunchProbe, /provider launch A\/B probe skipped: Path 1 uses the ordinary VM kernel\/provider boundary/)
  assert.match(providerLaunchProbe, /before checking or requiring Bubblewrap/)
  assert.match(providerLaunchProbe, /shared_host\|legacy_shared_host/)
  assert.doesNotMatch(installer, /usermod --append --groups chariox-slice chariox-docker/)
  assert.match(installer, /setfacl -P -m "u:chariox-docker:--x" -- "\$install_root\/var\/lib\/chariox-slice-share"/)
  assert.match(installer, /install -d -o chariox-docker -g chariox-slice -m 2710/)
  assert.match(installer, /rm -f -- "\$install_root\/etc\/systemd\/system\/multi-user\.target\.wants\/chariox-slice-broker\.service"/)
  assert.doesNotMatch(installer, /systemctl disable chariox-slice-broker\.service/)
  assert.match(accessHelper, /mapped_slice_uid=\$\(\(subuid_start \+ slice_uid - 1\)\)/)
  assert.match(accessHelper, /setfacl -P -R/)
  assert.doesNotMatch(accessHelper, /setfacl[^\n]*mapped_slice_uid[^\n]*-- "\$current"/)
  assert.match(managedBroker, /kind === "home_archive_capture"/)
  const archiveStream = await readFile(new URL("../apps/kernel/slice-linux-docker/managed-home-archive-stream.mjs", import.meta.url), "utf8")
  assert.doesNotMatch(managedBroker, /MAX_HOME_ARCHIVE_BYTES|maxBytes:/)
  assert.match(archiveStream, /HOME_ARCHIVE_MINIMUM_FREE_BYTES = policy\.minimumFreeBytes/)
  const archivePolicy = JSON.parse(await readFile(new URL("../apps/kernel/slice-linux-docker/home-archive-policy.json", import.meta.url), "utf8"))
  assert.deepEqual(archivePolicy, { schemaVersion: 1, minimumFreeBytes: 2 * 1024 ** 3, progressTimeoutMs: 300_000 })
  // Phase 1 capture admission: the kernel refuses save/backup unless the broker
  // verifies a protected layout, so the broker captures only that layout. The
  // protected capture runs as a lease-owned broker producer (release F).
  assert.match(managedBroker, /protectedLayouts\.requireQuiescedHome\(owner\)/)
  assert.match(managedBroker, /await spawnBounded\(process\.execPath,\s*\[join\(dirname\(fileURLToPath\(import\.meta\.url\)\), "protected-home-capture\.mjs"\)/)
  assert.match(managedBroker, /await digestPinnedHomeArchive\(archive\.fd, HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, brokerLifetime\.signal\)/)
  assert.match(managedBroker, /verifyManagedHomeArchive/)
  assert.match(managedBroker, /spawnControl\("\/usr\/bin\/mount", \["--bind", "\/proc\/self\/fd\/3", path\]/)
  assert.match(managedBroker, /spawnControl\("\/usr\/bin\/umount", \[path\]/)
  assert.doesNotMatch(managedBroker, /symlinkSync/)
  assert.doesNotMatch(managedBroker, /CHARIOX_SLICE_CLOUD_RELAY_CONFIG/)
})

test("managed image validation requires an explicit topology and preserves both paths", async () => {
  const [
    providerLaunchProbe,
    runbook,
    managedBootstrapState,
    imageReleaseVerifier,
    managedService,
    path1ManagedService,
  ] = await Promise.all([
    readFile(providerLaunchProbeUrl, "utf8"),
    readFile(runbookUrl, "utf8"),
    readFile(managedBootstrapStateUrl, "utf8"),
    readFile(imageReleaseVerifierUrl, "utf8"),
    readFile(managedServiceUrl, "utf8"),
    readFile(path1ManagedServiceUrl, "utf8"),
  ])

  for (const script of [providerLaunchProbe]) {
    assert.match(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
    assert.doesNotMatch(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1/)
    assert.match(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host/)
    assert.match(script, /path1/)
    assert.match(script, /shared_host/)
  }
  assert.match(path1ManagedService, /Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1/)
  assert.match(managedService, /Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=\/var\/lib\/chariox\/managed-bootstrap\.json/)
  assert.doesNotMatch(
    path1ManagedService,
    /^Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=/m,
    "Path-1 must resolve its bootstrap input through the protected kernel path",
  )
  assert.match(
    managedBootstrapState,
    /PROTECTED_MANAGED_BOOTSTRAP_PATH:[\s\S]*?"\/etc\/chariox\/bootstrap\/managed-bootstrap\.json"/,
  )
  assert.match(managedBootstrapState, /Path-1 managed bootstrap must use the protected bootstrap path/)
  assert.match(managedBootstrapState, /PathBuf::from\(PROTECTED_MANAGED_BOOTSTRAP_PATH\)/)
  assert.match(managedBootstrapState, /validate_protected_bootstrap_file\(path, PROTECTED_MANAGED_BOOTSTRAP_PATH\)/)
  assert.match(managedBootstrapState, /root:chariox with directory mode 0750 and file mode 0640/)
  assert.match(
    imageReleaseVerifier,
    /hasDataVolumeAdmission && lines\.some\(\(line\) => line\.startsWith\("Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH="\)\)/,
    "storage-capable Path-1 images must reject a bootstrap path environment override",
  )
  assert.match(path1ManagedService, /Environment=HOME=\/home\/chariox/)
  assert.match(path1ManagedService, /Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox/)
  assert.match(path1ManagedService, /^Environment=PATH=\/usr\/local\/sbin:\/usr\/local\/bin:\/usr\/sbin:\/usr\/bin:\/sbin:\/bin$/m)
  assert.match(path1ManagedService, /^ExecStart=\/usr\/local\/bin\/chariox-managed-bootstrap$/m)
  assert.doesNotMatch(path1ManagedService, /^UMask=/m, "Path-1 must use systemd's ordinary system-unit umask")
  for (const forbidden of [
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "bwrap",
    "NoNewPrivileges=",
    "PrivateTmp=",
    "ProtectSystem=",
    "ProtectHome=",
  ]) {
    assert.doesNotMatch(path1ManagedService, new RegExp(forbidden))
  }
  assert.match(
    providerLaunchProbe,
    /provider launch A\/B probe skipped: Path 1 uses the ordinary VM kernel\/provider boundary/,
  )

  const probePath = fileURLToPath(providerLaunchProbeUrl)
  const probeArguments = ["/nonexistent/chariox-rollback-kernel", "/nonexistent/chariox-candidate-kernel"]
  const runProbe = (topology) => {
    const env = { ...process.env }
    if (topology === undefined) delete env.CHARIOX_MANAGED_PROVIDER_TOPOLOGY
    else env.CHARIOX_MANAGED_PROVIDER_TOPOLOGY = topology
    return spawnSync(probePath, probeArguments, { encoding: "utf8", env })
  }

  const missing = runProbe(undefined)
  assert.equal(missing.status, 2, missing.stderr)
  assert.match(missing.stderr, /must be explicitly set to path1 or shared_host/)

  const invalid = runProbe("unknown-topology")
  assert.equal(invalid.status, 2, invalid.stderr)
  assert.match(invalid.stderr, /must be path1 or shared_host/)

  const path1 = runProbe("path1")
  assert.equal(path1.status, 0, path1.stderr)
  assert.match(path1.stderr, /probe skipped: Path 1/)

  const sharedHost = runProbe("shared_host")
  assert.notEqual(sharedHost.status, 0)
  assert.doesNotMatch(sharedHost.stderr, /probe skipped: Path 1/)
})

test("recovered managed development publications verify ACLs without rewriting content", async () => {
  const accessHelper = await readFile(publicationAccessUrl, "utf8")
  const runtimeState = await readFile(sliceDevelopmentRuntimeStateUrl, "utf8")
  const emptyDevelopment = await readFile(sliceEmptyDevelopmentUrl, "utf8")
  const accessDrill = await readFile(publicationAccessDrillUrl, "utf8")

  assert.match(accessHelper, /\[ "\$action" = verify \]/)
  assert.match(accessHelper, /verify_publication_access/)
  assert.match(runtimeState, /expected_publication\.is_some\(\).*"verify"/s)
  assert.match(emptyDevelopment, /expected\.is_some\(\).*"verify"/s)
  assert.doesNotMatch(
    accessHelper.match(/elif \[ "\$action" = verify \]; then(?<branch>.*?)\n  else/s)?.groups?.branch ?? "",
    /setfacl/,
  )
  assert.match(accessDrill, /"\$helper" verify/)
  assert.match(accessDrill, /\[ "\$before_verify" = "\$after_verify" \]/)
  assert.match(
    accessDrill,
    /runuser -u chariox -- sh -c 'umask 077; test "\$\(cat "\$1\/mapped-created"\)" = mapped; mkdir "\$1\/host-directory"'/,
  )
})

test("managed slice provider namespaces receive the required outer Docker compatibility policy", async () => {
  const provisioner = await readFile(sliceProvisionerUrl, "utf8")

  assert.match(provisioner, /--security-opt seccomp=unconfined/)
  assert.match(provisioner, /SLICE_APPARMOR_PROFILE="\$\{CHARIOX_SLICE_APPARMOR_PROFILE:-unconfined\}"/)
  assert.match(provisioner, /--security-opt apparmor="\$SLICE_APPARMOR_PROFILE"/)
  assert.match(provisioner, /--security-opt systempaths=unconfined/)
  assert.match(provisioner, /SLICE_APPARMOR_PROFILE.*\^\[A-Za-z0-9\]/)
  assert.match(provisioner, /CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP must be 0 or 1/)
  assert.match(provisioner, /CHARIOX_SLICE_ALLOW_PROVIDER_SANDBOX_COMPATIBILITY must be 0 or 1/)
})

test("Image snapshot labels preserve the complete release digest within provider limits", async () => {
  const runbook = await readFile(runbookUrl, "utf8")

  assert.match(runbook, /chariox\.dev\/runtime-release-a=<first 32 lowercase hex characters>/)
  assert.match(runbook, /chariox\.dev\/runtime-release-b=<last 32 lowercase hex characters>/)
  assert.match(runbook, /Concatenating `runtime-release-a` and\n`runtime-release-b` must reproduce/)
  assert.doesNotMatch(runbook, /chariox\.dev\/runtime-release=<64 lowercase hex characters>/)
})
