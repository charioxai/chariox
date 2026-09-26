import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { constants as fsConstants } from "node:fs"
import { access, chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const installSource = await readFile(new URL("./install-image.sh", import.meta.url), "utf8")
const upgradeSource = await readFile(new URL("./upgrade-image.sh", import.meta.url), "utf8")
const prepareSource = await readFile(new URL("./prepare-hetzner-image.sh", import.meta.url), "utf8")
const path1Service = await readFile(new URL("./chariox-path1-managed-bootstrap.service", import.meta.url), "utf8")
const disposableWorkerService = await readFile(new URL("./chariox-disposable-worker-bootstrap.service", import.meta.url), "utf8")
const providerResolverSources = await Promise.all(["codex", "claude", "opencode"].map((provider) =>
  readFile(new URL(`../../apps/kernel/src/provider/${provider}.rs`, import.meta.url), "utf8")))
const upgradeStateScript = new URL("./managed-kernel-upgrade-state.mjs", import.meta.url)

function indexOf(source, text, label, from = 0) {
  const index = source.indexOf(text, from)
  assert.notEqual(index, -1, `${label} must be present`)
  return index
}

function sourceFunction(source, name, nextName) {
  const start = indexOf(source, `fn ${name}(`, `${name} function`)
  const end = indexOf(source, `\nfn ${nextName}(`, `${nextName} function`, start)
  return source.slice(start, end)
}

test("Path-1 role units share the user-first provider executable path", async (context) => {
  const units = [path1Service, disposableWorkerService]
  const paths = units.map((unit) => {
    const assignments = unit.split(/\r?\n/).filter((line) => line.startsWith("Environment=PATH="))
    assert.equal(assignments.length, 1, "each Path-1 unit must set PATH once")
    return assignments[0].slice("Environment=PATH=".length).split(":")
  })
  assert.deepEqual(paths[0], paths[1])
  assert.deepEqual(paths[0], ["/home/chariox/.local/bin", "/usr/local/bin", "/usr/bin", "/bin"])

  const [codex, claude, opencode] = providerResolverSources
  assert.match(sourceFunction(codex, "resolve_candidate", "is_executable_file"),
    /env::split_paths\(&path_var\)[\s\S]*?\.find\(\|path\| is_executable_file\(path\)\)/)
  assert.match(sourceFunction(claude, "resolve_candidate", "is_executable_file"),
    /for directory in env::split_paths\(&path_var\)/)
  assert.match(sourceFunction(opencode, "resolve_candidate", "is_executable_file"),
    /env::split_paths\(&path_var\)[\s\S]*?\.find\(\|path\| is_executable_file\(path\)\)/)

  const scratch = await mkdtemp(join(tmpdir(), "chariox-path1-provider-path-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const fixtureDirectories = paths[0].map((_, index) => join(scratch, String(index)))
  await Promise.all(fixtureDirectories.map((directory) => mkdir(directory)))
  for (const provider of ["codex", "claude", "opencode"]) {
    const userExecutable = join(fixtureDirectories[0], provider)
    const imageExecutable = join(fixtureDirectories[1], provider)
    await Promise.all([
      writeFile(userExecutable, "user install", { mode: 0o755 }),
      writeFile(imageExecutable, "image install", { mode: 0o755 }),
    ])
    let selected
    for (const directory of fixtureDirectories) {
      const candidate = join(directory, provider)
      try {
        await access(candidate, fsConstants.X_OK)
        selected = candidate
        break
      } catch {}
    }
    assert.equal(selected, userExecutable, `${provider} must resolve the per-user install first`)
    await access(imageExecutable, fsConstants.X_OK)
  }
})

test("Path-1 role units do not inherit shared-host provider sandbox controls", () => {
  const forbidden = [
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
    "CHARIOX_SLICE_ROOT",
    "bwrap",
    "NoNewPrivileges=",
    "PrivateTmp=",
    "PrivateUsers=",
    "PrivateDevices=",
    "PrivateNetwork=",
    "ProtectSystem=",
    "ProtectHome=",
    "ProtectKernel",
    "ProtectControlGroups=",
    "RestrictNamespaces=",
    "RestrictAddressFamilies=",
    "RestrictSUIDSGID=",
    "ReadWritePaths=",
    "ReadOnlyPaths=",
    "InaccessiblePaths=",
    "BindPaths=",
    "BindReadOnlyPaths=",
    "RootDirectory=",
    "RootImage=",
    "SystemCallFilter=",
    "CapabilityBoundingSet=",
    "SupplementaryGroups=chariox-slice",
  ]
  for (const unit of [path1Service, disposableWorkerService]) {
    assert.ok(unit.includes("Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1"))
    for (const control of forbidden) assert.ok(!unit.includes(control), `Path-1 unit must omit ${control}`)
  }
})

test("install refuses to replace an invalid digest-named release", () => {
  const publishedBranch = installSource.slice(
    indexOf(installSource, 'if [ -e "$published_release" ] || [ -L "$published_release" ]; then', "published release check"),
    indexOf(installSource, 'if [ ! -e "$published_release" ]; then', "new release branch"),
  )

  assert.match(publishedBranch, /verify_selected_release/)
  assert.match(publishedBranch, /refus(?:e|ing).*immutable.*release/i)
  assert.match(publishedBranch, /exit 1/)
  assert.doesNotMatch(publishedBranch, /rm -rf.*published_release/)
})

test("install names releases from the validated manifest digest", () => {
  assert.ok(installSource.includes("release_name=${expected_release_digest#sha256:}"))
  assert.ok(installSource.includes("published_release=$releases_root/$release_name"))
  assert.ok(installSource.includes('verify_selected_release "$image_root" "$expected_release_digest" "$trusted_public_key"'))
  assert.ok(installSource.includes('verify_selected_release "$published_release" "$expected_release_digest" "$trusted_public_key"'))
  assert.ok(installSource.includes('"$@" path1 "$trusted_builder_public_key"'))
  assert.ok(upgradeSource.includes('"$@" path1 "$trusted_builder_public_key"'))
})

test("install and upgrade enforce immutable permissions after release signature verification", () => {
  for (const source of [installSource, upgradeSource]) {
    const start = indexOf(source, "verify_selected_release() {", "selected release verifier")
    const end = indexOf(source, "\n}", "selected release verifier end", start)
    const verifier = source.slice(start, end)
    const signatureLines = verifier.split("\n").filter((line) => line.includes('node "$script_root/verify-image-release.mjs"'))
    assert.ok(signatureLines.length > 0, "release signature verification must be present")
    assert.ok(signatureLines.every((line) => line.trimEnd().endsWith("|| return 1")),
      "signature failure must remain fatal when the wrapper is called conditionally")
    const signature = verifier.lastIndexOf('node "$script_root/verify-image-release.mjs"')
    const immutableTree = indexOf(
      verifier,
      'node "$script_root/managed-kernel-upgrade-state.mjs" verify-immutable-release-tree "$1" 0',
      "immutable release-tree verification",
    )
    assert.ok(signature < immutableTree, "permissions are checked after verifying the signed release")
  }
})

test("selected release verifier cannot mask a signature failure with a successful tree check", () => {
  for (const source of [installSource, upgradeSource]) {
    const start = indexOf(source, "verify_selected_release() {", "selected release verifier")
    const end = indexOf(source, "\n}", "selected release verifier end", start)
    const verifier = source.slice(start, end + 2)
    const command = [
      "set -e",
      "managed_provider_topology=shared_host",
      "service_name=chariox-managed-bootstrap.service",
      "script_root=/stub",
      "trusted_builder_public_key=unused",
      "node() { case \"$1\" in */verify-image-release.mjs) return 1 ;; */managed-kernel-upgrade-state.mjs) return 0 ;; *) return 2 ;; esac; }",
      verifier,
      "if verify_selected_release /tmp/release sha256:bad /tmp/key; then exit 22; fi",
      "exit 0",
    ].join("\n")
    const result = spawnSync("/bin/sh", ["-c", command], { encoding: "utf8" })
    assert.equal(result.status, 0, result.stderr)
  }
})

test("release-tree verification rejects group-writable signed content", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-release-tree-"))
  try {
    const context = join(root, "usr", "lib", "chariox", "slice-build-context")
    await mkdir(context, { recursive: true })
    const file = join(context, "tamperable-source")
    await writeFile(file, "signed but writable", { mode: 0o666 })
    await chmod(file, 0o666)

    const result = spawnSync(process.execPath, [
      upgradeStateScript.pathname,
      "verify-immutable-release-tree",
      root,
      String(process.getuid()),
    ], {
      encoding: "utf8",
    })
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /managed release tree grants group or other write access/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Path-1 install and upgrade keep the independent builder key available to runtime", () => {
  const keyPath = "/etc/chariox/trusted-builder-public-key"
  assert.ok(path1Service.includes(`Environment=CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=${keyPath}`))
  for (const source of [installSource, upgradeSource]) {
    assert.ok(source.includes(`trusted_builder_runtime_key=$install_root${keyPath}`))
    assert.match(source, /cmp -s \"\$trusted_builder_public_key\" \"\$trusted_builder_runtime_key\"/)
    assert.match(source, /install -o root -g root -m 0644 \"\$trusted_builder_public_key\" \"\$trusted_builder_runtime_key\"/)
  }
  const publishKey = indexOf(installSource, 'install -o root -g root -m 0644 "$trusted_builder_public_key" "$trusted_builder_runtime_key"', "runtime builder key publication")
  const activate = indexOf(installSource, 'atomic_symlink "releases/$release_name" "$install_root/usr/lib/chariox/current"', "current activation")
  assert.ok(publishKey < activate)
  assert.ok(prepareSource.includes('runtime_builder_key=/etc/chariox/trusted-builder-public-key'))
  assert.ok(prepareSource.includes('cmp -s "$CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY" "$runtime_builder_key"'))
})

test("install durably publishes the verified release before activating current", () => {
  const verify = indexOf(installSource, '"$pending_release" "$expected_release_digest" "$trusted_public_key"', "pending release verification")
  const syncTree = indexOf(installSource, 'sync-tree "$pending_release"', "release tree sync")
  const publish = indexOf(installSource, 'mv "$pending_release" "$published_release"', "release publication")
  const syncReleases = indexOf(installSource, 'sync-directory "$releases_root"', "release directory sync", publish)
  const activate = indexOf(installSource, 'atomic_symlink "releases/$release_name" "$install_root/usr/lib/chariox/current"', "current activation")

  assert.ok(verify < syncTree, "signature and digest verification precedes release syncing")
  assert.ok(syncTree < publish, "release contents are durable before digest-name publication")
  assert.ok(publish < syncReleases, "release-name publication is made durable")
  assert.ok(syncReleases < activate, "current is not activated before the release is durable")
})

test("install symlink replacement is atomic and durably synced", () => {
  const start = indexOf(installSource, "atomic_symlink() {", "atomic symlink helper")
  const end = indexOf(installSource, "releases_root=", "release installation")
  const helper = installSource.slice(start, end)

  assert.match(helper, /renameSync\(source, destination\)/)
  const rename = indexOf(helper, "renameSync(source, destination)", "atomic rename")
  const sync = indexOf(helper, 'sync-directory "$(dirname "$link_path")"', "parent directory sync", rename)
  assert.ok(rename < sync, "directory sync follows the atomic symlink rename")
})

test("install activation failure restores the prior current release", () => {
  const activation = installSource.slice(indexOf(installSource, 'atomic_symlink "releases/$release_name"', "current activation"))
  assert.ok(installSource.includes('atomic_symlink "$previous_current_target"'))
  assert.ok(activation.includes("if restore_previous_current; then"))
  assert.ok(activation.includes("managed release activation failed; restored previous current release"))
})

test("install rolls back if the current symlink or its directory sync fails", () => {
  assert.ok(installSource.includes("restore_previous_current() {"))
  const activation = installSource.slice(indexOf(installSource, 'if ! atomic_symlink "releases/$release_name"', "guarded current activation"))
  assert.ok(activation.includes("restore_previous_current"))
  assert.ok(activation.includes("current-link activation failed"))
})

test("upgrade durably publishes a verified release before creating its transaction", () => {
  const verify = indexOf(upgradeSource, '"$pending_release" "$expected_new_digest" "$next_trusted_public_key"', "pending release verification")
  const syncTree = indexOf(upgradeSource, 'sync-tree "$pending_release"', "pending release tree sync")
  const publish = indexOf(upgradeSource, 'mv "$pending_release" "$published_release"', "published release rename")
  const syncDirectory = indexOf(upgradeSource, 'sync-directory "$releases_root"', "release parent sync", publish)
  const transaction = indexOf(upgradeSource, 'pending_transaction=$chariox_root/.managed-kernel-upgrade.pending', "transaction preparation")

  assert.ok(verify < syncTree)
  assert.ok(syncTree < publish)
  assert.ok(publish < syncDirectory)
  assert.ok(syncDirectory < transaction)
})

test("upgrade verifies signed current, staged, and already-published releases before activation", () => {
  const currentVerify = indexOf(upgradeSource, '"$releases_root/${expected_current_digest#sha256:}" "$expected_current_digest" "$trusted_public_key"', "current release verification")
  const imageVerify = indexOf(upgradeSource, '"$image_root" "$expected_new_digest" "$next_trusted_public_key"', "incoming release verification")
  const publishedVerify = indexOf(upgradeSource, '"$published_release" "$expected_new_digest" "$next_trusted_public_key"', "published release verification")
  const stagedVerify = indexOf(upgradeSource, '"$pending_release" "$expected_new_digest" "$next_trusted_public_key"', "staged release verification")
  const activation = indexOf(upgradeSource, 'atomic_symlink "releases/$release_name" "$current_link"', "upgrade current activation")
  const transaction = indexOf(upgradeSource, 'pending_transaction=$chariox_root/.managed-kernel-upgrade.pending', "transaction preparation")
  const stop = indexOf(upgradeSource, 'if ! systemctl stop "$service_name"; then', "service stop", transaction)

  assert.ok(currentVerify < imageVerify)
  assert.ok(imageVerify < publishedVerify)
  assert.ok(imageVerify < stagedVerify)
  assert.ok(publishedVerify < activation)
  assert.ok(stagedVerify < activation)
  assert.ok(imageVerify < stop)
  assert.ok(stop < activation)
  assert.ok(upgradeSource.includes('node "$script_root/managed-kernel-upgrade-state.mjs" atomic-symlink "$1" "$2"'))
})

test("upgrade recovers interrupted phases and rolls back failed migration or health checks", () => {
  assert.ok(upgradeSource.includes("prepared|stopped|activated) rollback_transaction"))
  assert.ok(upgradeSource.includes("recover_transaction\nselect_receipt_path"))
  assert.ok(upgradeSource.includes('"$terminal_transaction/phase"'))
  assert.ok(upgradeSource.includes("if ! resume_home_migration; then"))
  assert.ok(upgradeSource.includes("managed kernel home migration failed; restored previous managed kernel release"))
  const healthFailure = upgradeSource.slice(indexOf(upgradeSource, '|| ! check_health "$target_protocol"', "target health check"))
  assert.ok(healthFailure.indexOf("rollback_transaction") > 0, "failed health checks trigger rollback")
  assert.ok(upgradeSource.includes('atomic_symlink "$previous_target" "$current_link"'))
  assert.ok(upgradeSource.includes("write_phase committed"))
  assert.ok(upgradeSource.includes("write_phase rolled_back"))
})

test("Path-1 upgrade checks both effective units before recovery and after reload", async (context) => {
  const guard = upgradeSource.match(/assert_path1_units_have_no_dropins\(\) \{\n[\s\S]*?^\}/m)?.[0]
  assert.ok(guard, "upgrade must inspect effective systemd drop-ins")

  const initialGuard = indexOf(upgradeSource, "assert_path1_units_have_no_dropins\nrecover_transaction", "pre-recovery guard")
  const transaction = indexOf(upgradeSource, "pending_transaction=$chariox_root/.managed-kernel-upgrade.pending", "transaction preparation")
  assert.ok(initialGuard < transaction, "drop-ins must block recovery and transaction writes")
  assert.match(upgradeSource, /systemctl daemon-reload \|\| return 1\n\s*assert_path1_units_have_no_dropins \|\| return 1\n[\s\S]*?systemctl start "\$service_name"/, "rollback must recheck after reload before starting")
  assert.match(upgradeSource, /if ! systemctl daemon-reload \\\n\s*\|\| ! assert_path1_units_have_no_dropins \\\n[\s\S]*?\|\| ! systemctl start "\$service_name"/, "activation must recheck after reload before starting")

  const scratch = await mkdtemp(join(tmpdir(), "chariox-upgrade-dropin-test-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const systemctl = join(scratch, "systemctl")
  await writeFile(systemctl, '#!/bin/sh\ncase "$*" in\n  *chariox-disposable-worker-bootstrap.service) printf "%s" "${SYSTEMD_WORKER_DROP_IN_PATHS:-}" ;;\n  *) printf "%s" "${SYSTEMD_HOME_DROP_IN_PATHS:-}" ;;\nesac\n')
  await chmod(systemctl, 0o755)
  const command = `${guard}\nassert_path1_units_have_no_dropins\n`
  const env = { ...process.env, PATH: `${scratch}:${process.env.PATH}`, managed_provider_topology: "path1" }
  for (const [name, home, worker, expectedExit] of [
    ["clean", "", "", 0],
    ["home drop-in", "/etc/systemd/system/home.d/50-hardening.conf", "", 1],
    ["worker drop-in", "", "/etc/systemd/system/worker.d/50-hardening.conf", 1],
  ]) {
    const result = spawnSync("/bin/sh", ["-c", command], {
      encoding: "utf8",
      env: { ...env, SYSTEMD_HOME_DROP_IN_PATHS: home, SYSTEMD_WORKER_DROP_IN_PATHS: worker },
    })
    assert.equal(result.status, expectedExit, `${name}: ${result.stderr}`)
    if (expectedExit) assert.match(result.stderr, /Path-1 service .* has systemd drop-ins/)
  }
})

test("release paths stay separate from mutable managed home state", () => {
  assert.ok(installSource.includes("managed_home=$install_root/home/chariox"))
  assert.ok(installSource.includes("managed_state=$managed_home/.chariox"))
  assert.ok(installSource.includes("releases_root=$install_root/usr/lib/chariox/releases"))
  assert.ok(upgradeSource.includes("managed_home=$install_root/home/chariox"))
  assert.ok(upgradeSource.includes("managed_state=$managed_home/.chariox"))
  assert.ok(upgradeSource.includes("releases_root=$chariox_root/releases"))
  assert.doesNotMatch(`${installSource}\n${upgradeSource}`, /rm -rf -- "\$managed_(?:home|state)(?:\/|"|\s)/)
})
