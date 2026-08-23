import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const scriptUrl = new URL("../deploy/managed-kernel/prepare-hetzner-image.sh", import.meta.url)
const versionsUrl = new URL("../deploy/managed-kernel/provider-versions.env", import.meta.url)
const publicationDockerfileUrl = new URL("../docker/publication/Dockerfile", import.meta.url)
const sliceDockerfileUrl = new URL("../apps/kernel/slice-linux-docker/docker/Dockerfile", import.meta.url)
const sliceToolchainPackageUrl = new URL("../apps/kernel/slice-linux-docker/toolchain/package.json", import.meta.url)
const sliceToolchainLockUrl = new URL("../apps/kernel/slice-linux-docker/toolchain/package-lock.json", import.meta.url)
const runbookUrl = new URL("../docs/MANAGED_REMOTE_KERNEL_IMAGE.md", import.meta.url)
const managedServiceUrl = new URL("../deploy/managed-kernel/chariox-managed-bootstrap.service", import.meta.url)
const rootlessServiceUrl = new URL("../deploy/managed-kernel/chariox-rootless-docker.service", import.meta.url)
const brokerServiceUrl = new URL("../deploy/managed-kernel/chariox-slice-broker.service", import.meta.url)
const installerUrl = new URL("../deploy/managed-kernel/install-image.sh", import.meta.url)
const publicationAccessUrl = new URL("../apps/kernel/slice-linux-docker/managed-publication-access.sh", import.meta.url)
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
  assert.match(script, /"\$script_root\/install-image\.sh" "\$release_rootfs" "\$release_digest" "\$trusted_public_key"/)
  assert.match(script, /systemctl is-enabled --quiet chariox-managed-bootstrap\.service/)
  assert.match(script, /systemctl is-active --quiet chariox-managed-bootstrap\.service/)
  assert.match(script, /managed runtime state entered the image/)
  assert.match(script, /rootless Docker state entered the image/)
  assert.match(script, /managed slice state entered the image/)
  assert.match(script, /broker output staging is not on the managed share filesystem/)
  assert.match(script, /npm_config_cache="\$npm_cache" npm ci --omit=dev/)
  assert.match(script, /rm -rf "\$npm_cache" \/root\/\.npm/)
  assert.match(script, /pnpm --version/)
  assert.match(script, /systemctl mask docker\.service docker\.socket/)
  assert.match(script, /configure_subid_range \/etc\/subuid --add-subuids/)
  assert.match(script, /configure_subid_range \/etc\/subgid --add-subgids/)
  assert.match(script, /requested subordinate ID range overlaps/)
  assert.match(script, /systemctl start chariox-rootless-docker\.service/)
  assert.match(script, /runuser -u chariox-docker -- env DOCKER_HOST=unix:\/\/\/run\/chariox-docker\/docker\.sock docker info/)
  assert.match(script, /runuser -u chariox -- env DOCKER_HOST=unix:\/\/\/run\/chariox-docker\/docker\.sock docker info/)
  assert.match(script, /systemctl stop chariox-rootless-docker\.service/)
  assert.match(script, /systemctl is-enabled --quiet chariox-rootless-docker\.service/)
  assert.match(script, /systemctl is-enabled --quiet chariox-slice-broker\.service/)
  assert.match(script, /slice Docker broker must be published only by managed bootstrap prestart/)
  assert.doesNotMatch(script, /usermod .*--groups docker chariox/)
  assert.doesNotMatch(script, /enable --now docker\.service/)
  assert.match(script, /cloud-init clean --logs --machine-id --seed/)
  assert.match(script, /rm -f \/etc\/ssh\/ssh_host_\* "\$MARKER_PATH"/)
  assert.doesNotMatch(script, /systemctl (?:start|restart|enable --now) chariox-managed-bootstrap/)
  assert.doesNotMatch(script, /\.arroba/)
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
    CHARIOX_CODEX_VERSION: "0.144.0",
    CHARIOX_OPENCODE_VERSION: "1.4.1",
    CHARIOX_CLAUDE_VERSION: "2.1.207",
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
  assert.doesNotMatch(dockerfile, /^# syntax=/m)
  assert.doesNotMatch(dockerfile, /npm install|rustup\.rs|deb\.nodesource\.com/)
  assert.deepEqual(toolchainPackage.dependencies, {
    "@anthropic-ai/claude-code": "2.1.207",
    "@openai/codex": "0.144.0",
    "opencode-ai": "1.4.1",
    pnpm: "11.22.0",
    ws: "8.18.3",
  })
  assert.equal(toolchainLock.lockfileVersion, 3)
  for (const [path, entry] of Object.entries(toolchainLock.packages)) {
    if (!path || entry.link) continue
    assert.match(entry.resolved ?? "", /^https:\/\/registry\.npmjs\.org\//, `${path} needs a registry artifact`)
    assert.match(entry.integrity ?? "", /^sha512-/, `${path} needs SHA-512 integrity`)
  }
})

test("managed Docker authority and publication access remain narrowly separated", async () => {
  const managed = await readFile(managedServiceUrl, "utf8")
  const rootless = await readFile(rootlessServiceUrl, "utf8")
  const broker = await readFile(brokerServiceUrl, "utf8")
  const installer = await readFile(installerUrl, "utf8")
  const accessHelper = await readFile(publicationAccessUrl, "utf8")
  const managedBroker = await readFile(managedBrokerUrl, "utf8")
  const rootlessNamespace = await readFile(rootlessNamespaceUrl, "utf8")

  assert.match(managed, /Wants=network-online\.target chariox-rootless-docker\.service/)
  assert.doesNotMatch(managed, /(?:Wants|After)=.*chariox-slice-broker/)
  assert.match(managed, /ExecStartPre=-\+\/usr\/bin\/systemctl restart chariox-slice-broker\.service/)
  assert.match(managed, /CHARIOX_CAPABILITY_ISOLATION_ROOT=\/var\/lib\/chariox\/home\/managed-context\/kernel/)
  assert.doesNotMatch(rootless, /SupplementaryGroups=chariox-slice/)
  assert.match(rootless, /Environment=PATH=\/usr\/local\/bin:\/usr\/bin:\/bin:\/usr\/sbin:\/sbin/)
  assert.match(rootless, /^ProtectKernelTunables=false$/m)
  assert.match(rootless, /--exec-opt native\.cgroupdriver=cgroupfs/)
  assert.match(rootless, /ReadWritePaths=.*\/var\/lib\/chariox-slice-share\/\.broker-private/)
  assert.match(rootless, /ReadWritePaths=.*\/var\/lib\/chariox-slice-share\/slices\/development/)
  assert.doesNotMatch(rootless, /ReadWritePaths=.*\/var\/lib\/chariox(?:\/home)?(?:\s|$)/)
  assert.match(broker, /^Restart=no$/m)
  assert.match(broker, /^Group=chariox-docker$/m)
  assert.doesNotMatch(broker, /^SupplementaryGroups=/m)
  assert.match(broker, /enter-rootless-docker-namespace\.sh \/usr\/bin\/node/)
  assert.match(broker, /CHARIOX_SLICE_DOCKER_BROKER_SOCKET=\/var\/lib\/chariox-slice-share\/\.broker-private\/control\/control\.sock/)
  assert.match(rootlessNamespace, /nsenter --target "\$child_pid" --user --mount/)
  assert.doesNotMatch(rootlessNamespace, /nsenter[^\n]*--net/)
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

test("managed slice provider namespaces receive the required outer Docker compatibility policy", async () => {
  const provisioner = await readFile(sliceProvisionerUrl, "utf8")

  assert.match(provisioner, /--security-opt seccomp=unconfined/)
  assert.match(provisioner, /--security-opt apparmor=unconfined/)
  assert.match(provisioner, /--security-opt systempaths=unconfined/)
  assert.match(provisioner, /CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP must be 0 or 1/)
})

test("Hetzner snapshot labels preserve the complete release digest within provider limits", async () => {
  const runbook = await readFile(runbookUrl, "utf8")

  assert.match(runbook, /chariox\.dev\/runtime-release-a=<first 32 lowercase hex characters>/)
  assert.match(runbook, /chariox\.dev\/runtime-release-b=<last 32 lowercase hex characters>/)
  assert.match(runbook, /Concatenating `runtime-release-a` and\n`runtime-release-b` must reproduce/)
  assert.doesNotMatch(runbook, /chariox\.dev\/runtime-release=<64 lowercase hex characters>/)
})
