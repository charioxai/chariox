import assert from "node:assert/strict"
import { once } from "node:events"
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn, spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const scriptUrl = new URL("../deploy/managed-kernel/prepare-hetzner-image.sh", import.meta.url)
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
const managedServiceUrl = new URL("../deploy/managed-kernel/chariox-managed-bootstrap.service", import.meta.url)
const path1ManagedServiceUrl = new URL("../deploy/managed-kernel/chariox-path1-managed-bootstrap.service", import.meta.url)
const workerServiceUrl = new URL("../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service", import.meta.url)
const workerKernelSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/worker.rs", import.meta.url)
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

test("Hetzner image preparation is pinned, guarded, and leaves no runtime identity", async () => {
  const script = await readFile(scriptUrl, "utf8")

  assert.match(script, /MARKER_VALUE=managed-remote-kernels-image-builder-v1/)
  assert.match(script, /refusing to modify a host that is not marked as the disposable image builder/)
  assert.match(script, /\[ "\$\(readlink "\$os_release"\)" = "\.\.\/usr\/lib\/os-release" \]/)
  assert.match(script, /os_release=\/usr\/lib\/os-release/)
  assert.match(script, /the image builder has no trusted regular os-release file/)
  assert.match(script, /\[ "\$\{ID:-\}" = "ubuntu" \]/)
  assert.match(script, /\[ "\$\{VERSION_ID:-\}" = "26\.04" \]/)
  assert.match(script, /\[ "\$node_major" -eq 22 \]/)
  assert.match(script, /grep -Eq '\^sha256:\[0-9a-f\]\{64\}\$'/)
  assert.match(script, /grep -Eq '\^\[0-9\]\+\\\.\[0-9\]\+\\\.\[0-9\]\+\$'/)
  assert.match(script, /provider-versions\.env/)
  assert.match(script, /provider_toolchain_source=\/usr\/lib\/chariox\/slice-build-context\/apps\/kernel\/slice-linux-docker\/toolchain/)
  assert.match(script, /npm ci --omit=dev/)
  assert.doesNotMatch(script, /npm install -g/)
  assert.match(script, /"\$script_root\/install-image\.sh" \\\s*"\$release_rootfs" "\$release_digest" "\$trusted_public_key" "\$managed_provider_topology"/)
  assert.match(script, /migration_journal=\/var\/lib\/chariox\/home-migration\.json/)
  assert.match(script, /migration_complete=\/var\/lib\/chariox\/home-migration-complete/)
  assert.match(script, /\.required == false and \.rootIdentity == null and \.entries == \[\]/)
  assert.match(script, /rm -f -- "\$migration_journal" "\$migration_complete"/)
  assert.match(script, /systemctl is-enabled --quiet "\$managed_bootstrap_service"/)
  assert.match(script, /systemctl is-active --quiet "\$managed_bootstrap_service"/)
  assert.match(script, /managed runtime state entered the image/)
  assert.match(script, /rootless Docker state entered the image/)
  assert.match(script, /managed slice state entered the image/)
  assert.match(script, /broker output staging is not on the managed share filesystem/)
  assert.match(script, /npm_config_cache="\$npm_cache" npm ci --omit=dev/)
  assert.match(script, /rm -rf "\$npm_cache" \/root\/\.npm/)
  assert.match(script, /chmod -R u=rwX,go=rX "\$provider_toolchain_root"/)
  assert.match(script, /runuser -u chariox -- env/)
  assert.match(script, /provider_tool_as_chariox codex --version/)
  assert.match(script, /provider_tool_as_chariox opencode --version/)
  assert.match(script, /provider_tool_as_chariox claude --version/)
  assert.match(script, /provider_tool_as_chariox pnpm --version/)
  assert.match(script, /pnpm --version/)
  assert.match(script, /systemctl mask docker\.service docker\.socket/)
  assert.match(script, /systemctl is-active docker\.service/)
  assert.match(script, /systemctl is-active docker\.socket/)
  assert.match(script, /remove-stale-rootful-docker-socket\.sh/)
  assert.match(script, /configure_subid_range \/etc\/subuid --add-subuids/)
  assert.match(script, /configure_subid_range \/etc\/subgid --add-subgids/)
  assert.match(script, /requested subordinate ID range overlaps/)
  assert.match(script, /systemctl start chariox-rootless-docker\.service/)
  assert.match(script, /^  dbus-user-session \\$/m)
  assert.match(script, /^  docker-buildx \\$/m)
  assert.match(script, /docker buildx version >\/dev\/null \|\| fail "Docker Buildx is unavailable"/)
  assert.match(script, /runuser -u chariox-docker -- env DOCKER_HOST=unix:\/\/\/run\/chariox-docker\/docker\.sock docker info/)
  assert.match(script, /slice_base_image=node:22\.17\.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c/)
  assert.match(script, /docker pull "\$slice_base_image"/)
  assert.match(script, /docker image inspect "\$slice_base_image"/)
  assert.match(script, /docker image rm "\$slice_base_image"/)
  assert.match(script, /runuser -u chariox -- env DOCKER_HOST=unix:\/\/\/run\/chariox-docker\/docker\.sock docker info/)
  assert.match(script, /systemctl stop chariox-rootless-docker\.service/)
  assert.match(script, /systemctl is-enabled --quiet chariox-rootless-docker\.service/)
  assert.match(script, /systemctl is-enabled --quiet chariox-slice-broker\.service/)
  assert.match(script, /slice Docker broker must be published only by managed bootstrap prestart/)
  assert.doesNotMatch(script, /usermod .*--groups docker chariox/)
  assert.doesNotMatch(script, /enable --now docker\.service/)
  assert.match(script, /cloud-init clean --logs --machine-id --seed/)
  assert.match(script, /passwd --lock root/)
  assert.match(script, /chage -d "\$\(date -u \+%Y-%m-%d\)" -M 99999 -I -1 -E -1 root/)
  assert.match(script, /PasswordAuthentication no/)
  assert.match(script, /KbdInteractiveAuthentication no/)
  assert.match(script, /PermitRootLogin prohibit-password/)
  assert.match(script, /sshd -T/)
  assert.match(script, /grep -Fxq 'permitrootlogin prohibit-password'/)
  assert.doesNotMatch(script, /grep -Fxq 'permitrootlogin without-password'/)
  assert.doesNotMatch(script, /install[^\n]*\/dev\/stdin/)
  assert.match(script, /managed_sshd_tmp=\$\(mktemp\)/)
  assert.match(
    script,
    /rm -rf \/var\/lib\/apt\/lists\/\* \/tmp\/chariox-managed-release \/root\/\.cache \/root\/\.npm \/root\/\.ssh/,
  )
  assert.match(script, /rm -f \/etc\/ssh\/ssh_host_\* "\$MARKER_PATH"/)
  assert.doesNotMatch(script, /systemctl (?:start|restart|enable --now) chariox-managed-bootstrap/)
  assert.doesNotMatch(script, /\.arroba/)
})

test("provider probes use a credential-free disposable home and remove it", async () => {
  const script = await readFile(scriptUrl, "utf8")
  const helper = script.match(/provider_tool_as_chariox\(\) \{(?<body>[\s\S]*?)\n\}/)?.groups?.body

  assert.ok(helper, "provider_tool_as_chariox helper must exist")
  assert.match(script, /provider_probe_home=\$\(mktemp -d \/tmp\/chariox-provider-probe\.XXXXXX\)/)
  assert.match(
    script,
    /provider_probe_home=\$\(mktemp -d \/tmp\/chariox-provider-probe\.XXXXXX\)\ntrap provider_probe_exit_cleanup 0/,
  )
  assert.match(script, /chown chariox:chariox "\$provider_probe_home"/)
  assert.match(script, /chmod 0700 "\$provider_probe_home"/)
  for (const signal of ["HUP", "INT", "TERM"]) {
    assert.match(script, new RegExp(`trap 'provider_probe_signal_cleanup ${signal}' ${signal}`))
  }
  assert.match(helper, /env -i/)
  assert.match(helper, /HOME="\$provider_probe_home"/)
  assert.match(helper, /PATH=\/usr\/local\/bin:\/usr\/bin:\/bin/)
  assert.match(helper, /sh -c 'cd "\$HOME" && exec "\$@"' sh "\$@"/)
  assert.match(
    script,
    /provider_tool_as_chariox pnpm --version[\s\S]*rm -rf "\$provider_probe_home"\ntrap - 0 HUP INT TERM[\s\S]*managed runtime state entered the image/,
  )
})

test("managed image proves private runtime files survive the masked temp namespace read-only", async () => {
  const prepare = await readFile(scriptUrl, "utf8")
  const probe = await readFile(providerRuntimeBindUrl, "utf8")

  assert.match(prepare, /sh "\$script_root\/verify-provider-runtime-bind\.sh"/)
  assert.match(probe, /probe_root=\$\(mktemp -d \/tmp\/chariox-provider-runtime-bind\.XXXXXX\)/)
  assert.match(probe, /chmod 0700 "\$probe_root"/)
  assert.match(probe, /chmod 0600 "\$probe_file"/)
  assert.match(probe, /bwrap=\/usr\/bin\/bwrap/)
  assert.match(probe, /stat -c %u "\$bwrap"/)
  assert.match(probe, /find "\$bwrap" -perm \/022/)
  assert.match(probe, /runuser -u chariox -- "\$bwrap"/)
  assert.match(probe, /--tmpfs \/tmp/)
  assert.match(probe, /--dir "\$probe_root"/)
  assert.match(probe, /--ro-bind "\$probe_root" "\$probe_root"/)
  assert.match(probe, /cat "\$probe_file"/)
  assert.match(probe, /mutation > "\$probe_file"/)
  assert.match(probe, /trap 'cleanup_signal TERM' TERM/)
})

test("provider probe cleanup preserves failures and termination signals", async () => {
  const script = await readFile(scriptUrl, "utf8")
  const cleanupBlock = script.match(
    /cleanup_provider_probe_home\(\) \{[\s\S]*?trap 'provider_probe_signal_cleanup TERM' TERM/,
  )?.[0]

  assert.ok(cleanupBlock, "provider probe cleanup and signal traps must exist")
  const fixtureRoot = await mkdtemp(join(tmpdir(), "chariox-provider-probe-cleanup-"))
  const harness = join(fixtureRoot, "cleanup-harness.sh")
  const dash = spawnSync("sh", ["-c", "command -v dash"], { encoding: "utf8" }).stdout.trim()
  const cleanupShell = dash || "sh"
  await writeFile(
    harness,
    `#!/bin/sh
set -eu
${cleanupBlock}
printf '%s\\n' "$provider_probe_home"
case $1 in
  fail) exit 37 ;;
  terminate) sleep 1 ;;
esac
`,
  )

  try {
    const failure = spawnSync(cleanupShell, [harness, "fail"], { encoding: "utf8" })
    assert.equal(failure.status, 37, failure.stderr)
    const failedProbeHome = failure.stdout.trim()
    assert.ok(failedProbeHome.startsWith("/tmp/chariox-provider-probe."))
    await assert.rejects(access(failedProbeHome))

    const termination = spawn(cleanupShell, [harness, "terminate"], {
      stdio: ["ignore", "pipe", "pipe"],
    })
    try {
      const [chunk] = await once(termination.stdout, "data")
      const terminatedProbeHome = chunk.toString().trim()
      assert.ok(terminatedProbeHome.startsWith("/tmp/chariox-provider-probe."))
      termination.kill("SIGTERM")
      const [exitCode, exitSignal] = await once(termination, "exit")
      assert.equal(exitCode, null)
      assert.equal(exitSignal, "SIGTERM")
      await assert.rejects(access(terminatedProbeHome))
    } finally {
      termination.kill("SIGKILL")
    }
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true })
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

test("Hetzner image preparation installs the hosted-drill tools", async () => {
  const script = await readFile(scriptUrl, "utf8")
  for (const dependency of [
    "acl",
    "bubblewrap",
    "build-essential",
    "ca-certificates",
    "cloud-init",
    "curl",
    "docker.io",
    "fuse-overlayfs",
    "gh",
    "git",
    "jq",
    "nodejs",
    "npm",
    "protobuf-compiler",
    "ripgrep",
    "rsync",
    "rootlesskit",
    "slirp4netns",
    "socat",
    "uidmap",
    "unzip",
    "util-linux",
    "zstd",
  ]) {
    assert.match(script, new RegExp(`\\n  ${dependency.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}(?: \\\\|\\n)`))
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
    CHARIOX_CODEX_VERSION: "0.144.5",
    CHARIOX_OPENCODE_VERSION: "1.18.23",
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

  for (const base of dockerfile.match(/^FROM\s+\S+/gm) ?? []) {
    assert.match(base, /@sha256:[a-f0-9]{64}$/)
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
    "@openai/codex": "0.144.5",
    "opencode-ai": "1.18.23",
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

test("managed Docker authority and publication access remain narrowly separated", async () => {
  const bootstrapEntrypoint = await readFile(bootstrapEntrypointUrl, "utf8")
  const managed = await readFile(managedServiceUrl, "utf8")
  const worker = await readFile(workerServiceUrl, "utf8")
  const workerKernel = await readFile(workerKernelSourceUrl, "utf8")
  const providerLaunchProbe = await readFile(providerLaunchProbeUrl, "utf8")
  const rootless = await readFile(rootlessServiceUrl, "utf8")
  const broker = await readFile(brokerServiceUrl, "utf8")
  const installer = await readFile(installerUrl, "utf8")
  const accessHelper = await readFile(publicationAccessUrl, "utf8")
  const managedBroker = await readFile(managedBrokerUrl, "utf8")
  const rootlessNamespace = await readFile(rootlessNamespaceUrl, "utf8")

  assert.match(bootstrapEntrypoint, /args\(\)\.any\(\|arg\| arg == "--disposable-worker"\)/)
  assert.match(
    bootstrapEntrypoint,
    /args\(\)\.any\(\|arg\| arg == "--disposable-worker"\)[\s\S]*worker::run_from_env\(\)/,
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
    "ProtectControlGroups=true",
    "RestrictSUIDSGID=true",
    "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
    "UMask=0007",
  ]) {
    assert.match(managed, new RegExp(`^${directive}$`, "m"))
  }
  assert.match(worker, /Environment=HOME=\/home\/chariox/)
  assert.match(worker, /Environment=CHARIOX_HOME=\/home\/chariox\/\.chariox/)
  assert.match(worker, /Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1/)
  assert.match(worker, /Environment=CHARIOX_DISPOSABLE_WORKER_RECEIPT=\/var\/lib\/chariox\/disposable-worker\/bootstrap-receipt\.json/)
  assert.match(worker, /Environment=CHARIOX_MANAGED_PROVIDER_HOME=\/var\/lib\/chariox\/provider-home/)
  assert.match(worker, /Environment=CHARIOX_MANAGED_VAULT_PATH=\/home\/chariox\/\.chariox\/vault\/vault\.json/)
  // The supervisor needs this endpoint to claim the one-shot broker. Its
  // child kernel receives only the scoped lease FD, never the socket path.
  assert.match(worker, /^Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=\/var\/lib\/chariox-slice-share\/\.broker-private\/control\/control\.sock$/m)
  assert.match(workerKernel, /for name in PATH1_SHARED_HOST_SELECTOR_ENVS\s*\{\s*command\.env_remove\(name\);/)
  assert.match(workerKernel, /super::supervisor::spawn_with_broker_lease\(&mut command\)/)
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
  assert.match(broker, /^Group=chariox-docker$/m)
  assert.doesNotMatch(broker, /^SupplementaryGroups=/m)
  assert.match(broker, /enter-rootless-docker-namespace\.sh \/usr\/bin\/node/)
  assert.match(broker, /CHARIOX_SLICE_DOCKER_BROKER_SOCKET=\/var\/lib\/chariox-slice-share\/\.broker-private\/control\/control\.sock/)
  assert.match(rootlessNamespace, /nsenter --target "\$child_pid" --user --mount/)
  assert.match(rootlessNamespace, /nsenter --target "\$child_pid" --user --mount --net -- "\$@"/)
  assert.match(rootlessNamespace, /\[ -L "\/proc\/\$child_pid\/ns\/net" \]/)
  const preparation = await readFile(scriptUrl, "utf8")
  assert.match(preparation, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
  assert.doesNotMatch(preparation, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1/)
  assert.match(preparation, /if \[ "\$managed_provider_topology" = shared_host \]; then[\s\S]*verify-provider-runtime-bind\.sh/)
  assert.match(preparation, /Bubblewrap remains installed for Docker-slice inner defense/)
  assert.match(providerLaunchProbe, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
  assert.doesNotMatch(providerLaunchProbe, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1/)
  assert.match(providerLaunchProbe, /provider launch A\/B probe skipped: Path 1 uses the ordinary VM kernel\/provider boundary/)
  assert.match(providerLaunchProbe, /before checking or requiring Bubblewrap/)
  assert.match(providerLaunchProbe, /shared_host\|legacy_shared_host/)
  assert.match(preparation, /"\$broker_namespace" docker build --pull --tag chariox-broker-network-check:local -/)
  assert.match(preparation, /DOCKER_BUILDKIT=1/)
  assert.match(preparation, /# syntax=docker\/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32/)
  assert.doesNotMatch(installer, /usermod --append --groups chariox-slice chariox-docker/)
  assert.match(installer, /setfacl -P -m "u:chariox-docker:--x" -- "\$install_root\/var\/lib\/chariox-slice-share"/)
  assert.match(installer, /install -d -o chariox-docker -g chariox-slice -m 2710/)
  assert.match(installer, /rm -f -- "\$install_root\/etc\/systemd\/system\/multi-user\.target\.wants\/chariox-slice-broker\.service"/)
  assert.doesNotMatch(installer, /systemctl disable chariox-slice-broker\.service/)
  assert.match(accessHelper, /mapped_slice_uid=\$\(\(subuid_start \+ slice_uid - 1\)\)/)
  assert.match(accessHelper, /setfacl -P -R/)
  assert.doesNotMatch(accessHelper, /setfacl[^\n]*mapped_slice_uid[^\n]*-- "\$current"/)
  assert.match(managedBroker, /kind === "home_archive_capture"/)
  assert.match(managedBroker, /MAX_HOME_ARCHIVE_BYTES = 32 \* 1024 \* 1024 \* 1024/)
  assert.match(managedBroker, /MIN_FREE_AFTER_ARCHIVE_BYTES = 2 \* 1024 \* 1024 \* 1024/)
  assert.match(managedBroker, /sha256sum/)
  assert.match(managedBroker, /verifyManagedHomeArchive/)
  assert.match(managedBroker, /spawnSync\("\/usr\/bin\/mount", \["--bind", "\/proc\/self\/fd\/3", path\]/)
  assert.match(managedBroker, /spawnSync\("\/usr\/bin\/umount", \[path\]/)
  assert.doesNotMatch(managedBroker, /symlinkSync/)
  assert.doesNotMatch(managedBroker, /CHARIOX_SLICE_CLOUD_RELAY_CONFIG/)
})

test("managed image validation requires an explicit topology and preserves both paths", async () => {
  const [preparation, providerLaunchProbe, runbook, path1ManagedService] = await Promise.all([
    readFile(scriptUrl, "utf8"),
    readFile(providerLaunchProbeUrl, "utf8"),
    readFile(runbookUrl, "utf8"),
    readFile(path1ManagedServiceUrl, "utf8"),
  ])

  assert.match(
    runbook,
    /sudo env CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host \\\n+  deploy\/managed-kernel\/prepare-hetzner-image\.sh \\\n+  <release-rootfs>/,
  )
  assert.match(runbook, /For a disposable-VM Path-1 image, replace `shared_host` with `path1` and set/)
  assert.match(runbook, /CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=<external-pinned-builder-public-key>/)

  for (const script of [preparation, providerLaunchProbe]) {
    assert.match(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
    assert.doesNotMatch(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1/)
    assert.match(script, /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host/)
    assert.match(script, /path1/)
    assert.match(script, /shared_host/)
  }
  assert.match(
    preparation,
    /if \[ "\$managed_provider_topology" = shared_host \]; then[\s\S]*verify-provider-runtime-bind\.sh/,
  )
  assert.match(preparation, /path1[\s\S]*managed_bootstrap_service=chariox-path1-managed-bootstrap\.service/)
  assert.match(preparation, /shared_host[\s\S]*managed_bootstrap_service=chariox-managed-bootstrap\.service/)
  assert.match(preparation, /other_managed_bootstrap_service/)
  assert.match(path1ManagedService, /Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1/)
  assert.match(path1ManagedService, /Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=\/var\/lib\/chariox\/managed-bootstrap\.json/)
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

test("Hetzner snapshot labels preserve the complete release digest within provider limits", async () => {
  const runbook = await readFile(runbookUrl, "utf8")

  assert.match(runbook, /chariox\.dev\/runtime-release-a=<first 32 lowercase hex characters>/)
  assert.match(runbook, /chariox\.dev\/runtime-release-b=<last 32 lowercase hex characters>/)
  assert.match(runbook, /Concatenating `runtime-release-a` and\n`runtime-release-b` must reproduce/)
  assert.doesNotMatch(runbook, /chariox\.dev\/runtime-release=<64 lowercase hex characters>/)
})
