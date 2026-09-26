import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

const homeServiceUrl = new URL("../deploy/managed-kernel/chariox-path1-managed-bootstrap.service", import.meta.url)
const workerServiceUrl = new URL("../deploy/managed-kernel/chariox-disposable-worker-bootstrap.service", import.meta.url)
const providerPathSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/provider_path.rs", import.meta.url)
const supervisorSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/supervisor.rs", import.meta.url)
const workerSourceUrl = new URL("../apps/kernel/src/managed_bootstrap/worker.rs", import.meta.url)
const bootstrapPath = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
const probeCommand = "printf '\\000CHARIOX_PROVIDER_PATH_V1\\000%s\\000' \"${PATH-}\""
const probeMarker = Buffer.from("\0CHARIOX_PROVIDER_PATH_V1\0")

function serviceEnvironment(unit, name) {
  const prefix = `Environment=${name}=`
  const lines = unit.split("\n").filter((line) => line.startsWith(prefix))
  assert.equal(lines.length, 1, `expected one ${name} assignment`)
  return lines[0].slice(prefix.length)
}

test("Path-1 provider PATH is resolved after verification without importing profile trust variables", async (context) => {
  const homeUnit = await readFile(homeServiceUrl, "utf8")
  const workerUnit = await readFile(workerServiceUrl, "utf8")
  for (const [unit, expectedCommand] of [
    [homeUnit, "ExecStart=/usr/local/bin/chariox-managed-bootstrap"],
    [workerUnit, "ExecStart=/usr/local/bin/chariox-managed-bootstrap --disposable-worker"],
  ]) {
    assert.equal(serviceEnvironment(unit, "PATH"), bootstrapPath)
    assert.ok(unit.split("\n").includes(expectedCommand), `missing direct command ${expectedCommand}`)
    assert.doesNotMatch(unit, /--login|EnvironmentFile=|\.profile|\.bashrc/)
  }

  const [providerPathSource, supervisorSource, workerSource] = await Promise.all([
    readFile(providerPathSourceUrl, "utf8"),
    readFile(supervisorSourceUrl, "utf8"),
    readFile(workerSourceUrl, "utf8"),
  ])
  assert.match(providerPathSource, /\.env_clear\(\)/)
  assert.match(providerPathSource, /libc::poll/)
  assert.match(providerPathSource, /PROBE_TIMEOUT/)
  assert.match(providerPathSource, /MAX_PROBE_OUTPUT_BYTES/)
  assert.match(providerPathSource, /MAX_PROVIDER_PATH_BYTES/)
  assert.match(providerPathSource, /parse_login_path\(&output\)/)
  const supervisorLaunchStart = supervisorSource.indexOf("fn spawn_kernel_with_handoff(")
  const supervisorLaunchEnd = supervisorSource.indexOf("\nfn configured_managed_slice_boundary(", supervisorLaunchStart)
  assert.ok(supervisorLaunchStart >= 0 && supervisorLaunchEnd > supervisorLaunchStart)
  const supervisorLaunchSource = supervisorSource.slice(supervisorLaunchStart, supervisorLaunchEnd)
  const workerLaunchStart = workerSource.indexOf("fn spawn_kernel(")
  const workerLaunchEnd = workerSource.indexOf(
    '\n#[cfg(target_os = "linux")]\nfn prepare_disposable_worker_provider_home',
    workerLaunchStart,
  )
  assert.ok(workerLaunchStart >= 0 && workerLaunchEnd > workerLaunchStart)
  const workerLaunchSource = workerSource.slice(workerLaunchStart, workerLaunchEnd)
  const verifiedPathLaunch = /release: &VerifiedRelease,[\s\S]*?resolve_login_path\(\s*&config\.process_home\s*,?\s*\)[\s\S]*?\.env\("PATH",\s*provider_path\)/
  assert.match(supervisorLaunchSource, verifiedPathLaunch)
  assert.match(
    workerLaunchSource,
    /release: &VerifiedRelease,[\s\S]*?resolve_worker_login_path\(\s*&config\.process_home\s*,?\s*\)[\s\S]*?\.env\("PATH",\s*provider_path\)/,
  )
  assert.ok(
    supervisorLaunchSource.indexOf("resolve_login_path") <
      supervisorLaunchSource.indexOf("prepare_kernel_local_auth_file(config)"),
    "resolve the provider PATH before preparing local-auth credentials",
  )
  assert.match(
    providerPathSource,
    /const BOOTSTRAP_PATH: &str = "\/usr\/local\/sbin:\/usr\/local\/bin:\/usr\/sbin:\/usr\/bin:\/sbin:\/bin";/,
  )
  assert.match(
    providerPathSource,
    /pub\(super\) fn resolve_login_path\(home: &Path\) -> OsString \{\s*resolve_login_path_with_fallback\(home, false\)/,
  )
  assert.match(
    providerPathSource,
    /pub\(super\) fn resolve_worker_login_path\(home: &Path\) -> OsString \{\s*resolve_login_path_with_fallback\(home, true\)/,
  )
  assert.match(
    providerPathSource,
    /fn fallback_path\(home: &Path, include_home_local_bin: bool\)[\s\S]*?home\.join\("\.local\/bin"\)[\s\S]*?is_safe_path_component\(component\)[\s\S]*?format!\("\{local_bin\}:\{BOOTSTRAP_PATH\}"\)[\s\S]*?BOOTSTRAP_PATH\.to_string\(\)/,
  )
  assert.match(
    providerPathSource,
    /Err\(reason\) => \{\s*let fallback_path = fallback_path\(home, include_home_local_bin\);[\s\S]*?crate::logging::warn_with_fields\(\s*"managed_bootstrap\.provider_path_probe_failed"[\s\S]*?"fallback_path": fallback_path\.as_str\(\),[\s\S]*?OsString::from\(fallback_path\)/,
  )

  const fixtureHome = await mkdtemp(join(tmpdir(), "chariox-provider-path-security-"))
  context.after(() => rm(fixtureHome, { recursive: true, force: true }))
  const localBin = join(fixtureHome, ".local", "bin")
  await mkdir(localBin, { recursive: true })
  const expectedProviderPath = `${localBin}:${bootstrapPath}`
  await writeFile(join(fixtureHome, ".profile"), [
    "export CHARIOX_MANAGED_RELEASE_PUBLIC_KEY=/profile/untrusted-release-key",
    "export CHARIOX_MANAGED_KERNEL_BINARY=/profile/untrusted-kernel",
    "export CHARIOX_MANAGED_BOOTSTRAP_PATH=/profile/untrusted-bootstrap.json",
    "export CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host",
    "export CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=/profile/untrusted-builder-key",
    "export LD_PRELOAD=/profile/hostile.so",
    'export PATH="$HOME/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"',
    "printf 'provider profile banner\\n'",
    "",
  ].join("\n"))
  const providerTool = join(localBin, "chariox-custom-provider-tool")
  await writeFile(providerTool, [
    "#!/bin/sh",
    "printf 'tool=custom-provider-tool\\n'",
    "printf 'release_key=%s\\n' \"${CHARIOX_MANAGED_RELEASE_PUBLIC_KEY-<unset>}\"",
    "printf 'kernel_binary=%s\\n' \"${CHARIOX_MANAGED_KERNEL_BINARY-<unset>}\"",
    "printf 'bootstrap_path=%s\\n' \"${CHARIOX_MANAGED_BOOTSTRAP_PATH-<unset>}\"",
    "printf 'topology=%s\\n' \"${CHARIOX_MANAGED_PROVIDER_TOPOLOGY-<unset>}\"",
    "printf 'builder_key=%s\\n' \"${CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY-<unset>}\"",
    "printf 'ld_preload=%s\\n' \"${LD_PRELOAD-<unset>}\"",
    "",
  ].join("\n"))
  await chmod(providerTool, 0o700)

  const probe = spawnSync("/bin/bash", ["--login", "-c", probeCommand], {
    cwd: "/",
    encoding: "buffer",
    env: {
      HOME: fixtureHome,
      USER: "chariox",
      LOGNAME: "chariox",
      SHELL: "/bin/bash",
      PATH: bootstrapPath,
    },
    maxBuffer: 8192,
    timeout: 5000,
  })
  assert.equal(probe.status, 0, probe.stderr?.toString() ?? probe.error?.message)
  const markerOffset = probe.stdout.lastIndexOf(probeMarker)
  assert.notEqual(markerOffset, -1)
  assert.equal(probe.stdout.at(-1), 0)
  const capturedPath = probe.stdout.subarray(markerOffset + probeMarker.length, -1).toString("utf8")
  assert.equal(capturedPath, expectedProviderPath)

  const child = spawnSync("chariox-custom-provider-tool", [], {
    encoding: "utf8",
    env: {
      HOME: fixtureHome,
      PATH: capturedPath,
      CHARIOX_MANAGED_RELEASE_PUBLIC_KEY: "/usr/lib/chariox/release-public-key",
      CHARIOX_MANAGED_KERNEL_BINARY: "/usr/local/bin/chariox-kernel",
      CHARIOX_MANAGED_BOOTSTRAP_PATH: "/var/lib/chariox/managed-bootstrap.json",
      CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: "/etc/chariox/trusted-builder-public-key",
    },
  })
  assert.equal(child.status, 0, child.error?.message ?? child.stderr)
  assert.equal(child.stdout, [
    "tool=custom-provider-tool",
    "release_key=/usr/lib/chariox/release-public-key",
    "kernel_binary=/usr/local/bin/chariox-kernel",
    "bootstrap_path=/var/lib/chariox/managed-bootstrap.json",
    "topology=path1",
    "builder_key=/etc/chariox/trusted-builder-public-key",
    "ld_preload=<unset>",
    "",
  ].join("\n"))
})
