import { spawnOwned, signalOwnedProcessGroup } from "../apps/kernel/slice-linux-docker/owned-process-signals.mjs"
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import { existsSync, readdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { once } from "node:events"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

// install-root.sh against a fake root (CHARIOX_LOCAL_INSTALL_ROOT) with stub
// system commands; the real runtime installer is replaced by a stub that
// accepts only the trusted key. The live install is drilled on a real host.
const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const installer = join(repositoryRoot, "deploy/local-linux/install-root.sh")
const linux = process.platform === "linux"
const KEY = "a".repeat(64)
const users = { root: 0, alice: 1000, bob: 1001, "Carol.Smith": 1002, rootgroup: 1003 }
const groups = { ...users, rootgroup: 0 }

async function script(path, contents) {
  await writeFile(path, contents)
  await chmod(path, 0o755)
}

const DESKTOP = ["Xvfb", "openbox", "bwrap", "dbus-daemon", "setxkbmap"]
const PACKAGES = "python3-xlib python3-pil python3-gi python3-pyatspi at-spi2-core xvfb openbox bubblewrap dbus-daemon x11-xkb-utils libasound2 libatk-bridge2.0-0 libatk1.0-0 libatspi2.0-0 libcairo2 libcups2 libdbus-1-3 libdrm2 libgbm1 libglib2.0-0 libnspr4 libnss3 libpango-1.0-0 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxtst6 libxfixes3 libxkbcommon0 libxrandr2"

async function harness() {
  const base = await mkdtemp(join(tmpdir(), "chariox-local-install-"))
  const root = join(base, "root")
  const bin = join(base, "bin")
  const pkg = join(base, "package")
  const state = join(base, "state")
  for (const dir of [bin, pkg, state, join(root, "sys/fs/cgroup"), join(root, "sys/module/apparmor/parameters"),
    join(root, "sys/kernel/security/apparmor"), join(root, "proc/sys/kernel"), join(root, "var/lib/systemd/linger")]) {
    await mkdir(dir, { recursive: true })
  }
  await writeFile(join(root, "sys/fs/cgroup/cgroup.controllers"), "cpu memory pids\n")
  await writeFile(join(root, "sys/module/apparmor/parameters/enabled"), "Y\n")
  await writeFile(join(root, "proc/sys/kernel/apparmor_restrict_unprivileged_userns"), "1\n")
  await writeFile(join(root, "sys/kernel/security/apparmor/profiles"), "")
  const passwd = Object.entries(users).map(([name, uid]) => `${name}:x:${uid}:${groups[name]}::/home/${name}:/bin/bash`)
  await mkdir(join(root, "usr/bin"), { recursive: true })
  await script(join(root, "usr/bin/python3"), `#!/bin/sh
[ "\${HARNESS_NATIVE_PYTHON:-ready}" = ready ] || test -e "$HARNESS_STATE/native-python-ready"
`)
  await mkdir(join(root, "etc/at-spi2"), { recursive: true })
  await writeFile(join(root, "etc/at-spi2/accessibility.conf"), "<busconfig/>")
  for (const name of DESKTOP) await script(join(root, "usr/bin", name), "#!/bin/sh\n")
  await script(join(root, "usr/bin/apt-get"), `#!/bin/sh
echo "$*" >> "$HARNESS_STATE/native-apt"
[ "\${HARNESS_NATIVE_APT_FAIL:-0}" = 0 ] || exit 42
case "$1" in install) [ "\${HARNESS_NATIVE_STAYS_BROKEN:-0}" = 1 ] || { touch "$HARNESS_STATE/native-python-ready"; for b in ${DESKTOP.join(" ")}; do printf '#!/bin/sh\n' > "$HARNESS_ROOT/usr/bin/$b"; chmod 755 "$HARNESS_ROOT/usr/bin/$b"; done; } ;; esac
`)
  // This fixture owns the system dependency inventory; do not consult the
  // runner's ldconfig cache or require its Chromium libraries to be installed.
  const libraries = ["libasound.so.2", "libatk-1.0.so.0", "libatk-bridge-2.0.so.0", "libatspi.so.0", "libcups.so.2", "libgbm.so.1", "libnspr4.so", "libnss3.so", "libxkbcommon.so.0", "libXcomposite.so.1", "libXdamage.so.1", "libXrandr.so.2", "libX11.so.6", "libXext.so.6", "libXtst.so.6"]
  await script(join(bin, "ldconfig"), `#!/bin/sh\nprintf '%s\\n' ${libraries.map(name => `'${name}'`).join(" ")}\n`)
  await script(join(bin, "id"), `#!/bin/sh
case "$*" in
  -u) echo "\${HARNESS_UID:-0}" ;;
${Object.entries(users).map(([name, uid]) => `  "-u ${name}") echo ${uid} ;;\n  "-g ${name}") echo ${groups[name]} ;;`).join("\n")}
  *) exit 1 ;;
esac
`)
  await script(join(bin, "getent"), `#!/bin/sh
printf '%s\\n' ${passwd.map((line) => `'${line}'`).join(" ")} | awk -F: -v key="$2" '$1 == key || $3 == key'
`)
  await script(join(bin, "systemctl"), `#!/bin/sh
echo "$*" >> "$HARNESS_STATE/systemctl"
case "$*" in
  "show chariox-app-storage.service -p MainPID --value") echo 123 ;;
  "show user@"*) echo "\${HARNESS_DELEGATE:-cpu memory pids}" ;;
  "is-enabled --quiet chariox-docker-admission-locks.service") test -e "$HARNESS_STATE/enabled-admission" ;;
  "enable --quiet chariox-docker-admission-locks.service") touch "$HARNESS_STATE/enabled-admission" ;;
  "is-enabled --quiet chariox-app-storage.service") test -e "$HARNESS_STATE/enabled" ;;
  "is-active --quiet chariox-app-storage.service") test -e "$HARNESS_STATE/active" ;;
  "enable --quiet chariox-app-storage.service") touch "$HARNESS_STATE/enabled" ;;
  "start chariox-app-storage.service"|"restart chariox-app-storage.service") mkdir -p "$CHARIOX_LOCAL_INSTALL_ROOT/proc/123"; cp "$CHARIOX_LOCAL_INSTALL_ROOT/usr/libexec/chariox-app-storage" "$CHARIOX_LOCAL_INSTALL_ROOT/proc/123/exe"; touch "$HARNESS_STATE/active" ;;
  "stop chariox-app-storage.service") rm -f "$HARNESS_STATE/active" ;;
  "disable --now --quiet chariox-app-storage.service") rm -f "$HARNESS_STATE/active" "$HARNESS_STATE/enabled" ;;
  daemon-reload) ;;
  *) exit 1 ;;
esac
`)
  await script(join(bin, "loginctl"), `#!/bin/sh
echo "$*" >> "$HARNESS_STATE/loginctl"
case "$1" in
  enable-linger) touch "$CHARIOX_LOCAL_INSTALL_ROOT/var/lib/systemd/linger/$2" ;;
  disable-linger) rm -f "$CHARIOX_LOCAL_INSTALL_ROOT/var/lib/systemd/linger/$2" ;;
esac
`)
  await script(join(bin, "apparmor_parser"), `#!/bin/sh
echo "$*" >> "$HARNESS_STATE/apparmor"
profiles="$CHARIOX_LOCAL_INSTALL_ROOT/sys/kernel/security/apparmor/profiles"
case "$1" in -r) printf "chariox-app-bwrap (unconfined)\\nchariox-app-domain-entry (unconfined)\\nchariox-browser-unshare (unconfined)\\nchariox-browser-chromium (unconfined)\\n" > "$profiles" ;; -R) : > "$profiles" ;; esac
`)
  // Ownership is the only thing a non-root test cannot give; the rest is real install(1).
  await script(join(bin, "install"), `#!/bin/sh
for arg do
  shift
  if [ -n "$skip" ]; then skip=; continue; fi
  case "$arg" in -o|-g) skip=1 ;; *) set -- "$@" "$arg" ;; esac
done
exec /usr/bin/install "$@"
`)
  // The pinned kernel Chromium: a small archive of the same layout, never the real download.
  const archive = join(base, "chrome-linux64.zip")
  const zipped = spawnSync("python3", ["-c", `import sys, zipfile
z = zipfile.ZipFile(sys.argv[1], "w")
exe = zipfile.ZipInfo("chrome-linux64/chrome"); exe.external_attr = 0o100755 << 16
z.writestr(exe, "#!/bin/sh\\necho chromium 1.2.3\\n")
z.writestr("chrome-linux64/resources.pak", "data")
z.close()`, archive])
  assert.equal(zipped.status, 0, zipped.stderr?.toString())
  const archiveBytes = await readFile(archive)
  const browserLock = join(base, "kernel-browser-chromium.lock.json")
  const browserPin = (sha256 = createHash("sha256").update(archiveBytes).digest("hex")) => writeFile(browserLock, JSON.stringify({ schema: "chariox.kernel-browser-chromium.v1", distribution: "fixture", version: "1.2.3",
    linux64: { url: `file://${archive}`, sha256, size: archiveBytes.length, executable: "chrome-linux64/chrome" } }))
  await browserPin()
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, CHARIOX_LOCAL_INSTALL_ROOT: root, HARNESS_ROOT: root, HARNESS_STATE: state, CHARIOX_LOCAL_INSTALL_LDCONFIG: join(bin, "ldconfig"), CHARIOX_LOCAL_INSTALL_BROWSER_LOCK: browserLock }
  const run = (args, extra = {}) => {
    const result = spawnSync("bash", [installer, ...args], { env: { ...env, ...extra }, encoding: "utf8" })
    return { ...result, out: result.stdout + result.stderr }
  }
  const install = (who, extra = [], env = {}) =>
    run(["install", ...who.flatMap((name) => ["--user", name]), ...extra], env)

  const log = async (name) => (existsSync(join(state, name)) ? readFile(join(state, name), "utf8") : "")
  const reset = (name) => rm(join(state, name), { force: true })
  return { base, root, bin, pkg, run, install, log, reset, browserPin }
}

const p = (h, path) => join(h.root, path)

test("local Linux root install refuses GID zero before changing shared enrollment", { skip: !linux }, async () => {
  const h = await harness()
  try {
    const alice = h.install(["alice"])
    assert.equal(alice.status, 0, alice.out)
    const logs = Object.fromEntries(await Promise.all(
      ["runtime-install", "systemctl", "loginctl", "apparmor"].map(async (name) => [name, await h.log(name)]),
    ))
    const result = h.install(["bob", "rootgroup"])
    assert.equal(result.status, 1)
    assert.match(result.out, /rootgroup must have a non-root primary group/)
    assert.ok(!existsSync(p(h, "etc/chariox/app-storage.json")))
    for (const [name, contents] of Object.entries(logs)) assert.equal(await h.log(name), contents)
    assert.equal(existsSync(p(h, "var/lib/systemd/linger/1001")), false)
    assert.equal(existsSync(p(h, "var/lib/systemd/linger/1003")), false)
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})

// Use the real parser: the installer harness deliberately stubs profile loading.
test("shipped AppArmor attachments compile with the real parser", { skip: !linux }, (t) => {
  const available = spawnSync("apparmor_parser", ["--version"], { encoding: "utf8" })
  if (available.error?.code === "ENOENT") return t.skip("apparmor_parser unavailable")
  const result = spawnSync("apparmor_parser", ["-Q", "--skip-cache", join(repositoryRoot, "deploy/local-linux/chariox-app-bwrap.apparmor")], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr || result.stdout)
})


test("AppArmor without unprivileged userns restriction does not load an ABI 4 exception", { skip: !linux }, async () => {
  const h = await harness()
  try {
    await rm(p(h, "proc/sys/kernel/apparmor_restrict_unprivileged_userns"))
    // Bookworm's AppArmor 3 parser cannot compile the ABI 4 userns profile.
    await script(join(h.bin, "apparmor_parser"), `#!/bin/sh
 echo "$*" >> "$HARNESS_STATE/apparmor"
 echo "Could not open abi/4.0" >&2
 exit 1
`)
    const result = h.install(["alice"])
    assert.equal(result.status, 0, result.out)
    assert.equal(await h.log("apparmor"), "")
    assert.equal(existsSync(p(h, "etc/apparmor.d/chariox-app-bwrap")), false)
    assert.match(await h.log("loginctl"), /^enable-linger alice$/m)
    // The old installer wrote this file before the ABI 4 parser failure.
    const profile = p(h, "etc/apparmor.d/chariox-app-bwrap")
    await mkdir(join(profile, ".."), { recursive: true })
    await writeFile(profile, await readFile(join(repositoryRoot, "deploy/local-linux/chariox-app-bwrap.apparmor")))
    const dry = h.install(["alice"], ["--dry-run"])
    assert.equal(dry.status, 0, dry.out)
    assert.ok(existsSync(profile))
    const recovered = h.install(["alice"])
    assert.equal(recovered.status, 0, recovered.out)
    assert.equal(existsSync(profile), false)
    assert.equal(await h.log("apparmor"), "")
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})


test("restricted kernel with an incompatible AppArmor parser fails actionably before installation", { skip: !linux }, async () => {
  const h = await harness()
  try {
    await script(join(h.bin, "apparmor_parser"), `#!/bin/sh
 echo "$*" >> "$HARNESS_STATE/apparmor"
 echo "Could not open abi/4.0" >&2
 exit 1
`)
    const result = h.install(["alice"])
    assert.equal(result.status, 1, result.out)
    assert.match(result.out, /AppArmor 4.*parser.*ABI 4.*re-run/)
    assert.ok(!existsSync(p(h, "usr/lib/chariox/browser/chariox-browser-unshare")))
    assert.equal(await h.log("runtime-install"), "")
    assert.doesNotMatch(await h.log("systemctl"), /^start |^restart |^enable /m)
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})



test("a pinned Chromium whose bytes differ from the pin is refused before anything is installed", { skip: !linux }, async () => {
  const h = await harness()
  try {
    await h.browserPin("0".repeat(64))
    const refused = h.install(["alice"])
    assert.equal(refused.status, 1, refused.out)
    assert.match(refused.out, /has SHA-256 [0-9a-f]{64} \(\d+ bytes\), not the pinned 0{64}/)
    assert.equal(existsSync(p(h, "usr/lib/chariox/browser/chromium/1.2.3")), false)
    assert.equal(existsSync(p(h, "etc/chariox/app-storage.json")), false, "refused before the first change")
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})

// MP-08/MP-10/MP-11: native keyboard startup uses the SYSTEM Python, not a user venv.
test("local Linux Setup installs and verifies missing native keyboard dependencies", { skip: !linux }, async () => {
  const h = await harness()
  try {
    const preview = h.install(["alice"], ["--dry-run"], { HARNESS_NATIVE_PYTHON: "missing" })
    assert.equal(preview.status, 0, preview.out)
    assert.equal(await h.log("native-apt"), "", "dry-run cannot install host packages")
    const result = h.install(["alice"], [], { HARNESS_NATIVE_PYTHON: "missing" })
    assert.equal(result.status, 0, result.out)
    assert.equal(await h.log("native-apt"), `update\ninstall -y --no-install-recommends ${PACKAGES}\n`)
    await h.reset("native-apt")
    const again = h.install(["alice"], [], { HARNESS_NATIVE_PYTHON: "missing" })
    assert.equal(again.status, 0, again.out)
    assert.equal(await h.log("native-apt"), "", "already-importable system modules need no package operation")
  } finally { await rm(h.base, { recursive: true, force: true }) }
})

// MP-08/MP-10/MP-11: a stock Ubuntu 26.04 host has no openbox; the kernel's owned desktop refused to start.
test("local Linux Setup installs the owned desktop's missing system executables", { skip: !linux }, async () => {
  const h = await harness()
  try {
    await rm(p(h, "usr/bin/openbox"))
    const result = h.install(["alice"])
    assert.equal(result.status, 0, result.out)
    assert.equal(await h.log("native-apt"), `update\ninstall -y --no-install-recommends ${PACKAGES}\n`)
    assert.equal(existsSync(p(h, "usr/bin/openbox")), true)
  } finally { await rm(h.base, { recursive: true, force: true }) }
})

test("local Linux Setup refuses failed or unverifiable native dependencies", { skip: !linux }, async () => {
  for (const failure of [{ HARNESS_NATIVE_APT_FAIL: "1" }, { HARNESS_NATIVE_STAYS_BROKEN: "1" }]) {
    const h = await harness()
    try {
      const result = h.install(["alice"], [], { HARNESS_NATIVE_PYTHON: "missing", ...failure })
      assert.notEqual(result.status, 0, result.out)
      assert.match(result.out, /browser dependenc/i)
    } finally { await rm(h.base, { recursive: true, force: true }) }
  }
})

// MP-08 / MP-09 / MP-10 / MP-11: the root step owns browser dependencies only.
test("WASM Apps need no root runtime bundle or storage enrollment", { skip: !linux }, async () => {
  const h = await harness()
  try {
    const first = h.install(["alice"])
    assert.equal(first.status, 0, first.out)
    assert.ok(existsSync(p(h, "usr/lib/chariox/browser/chariox-browser-unshare")))
    assert.ok(!existsSync(p(h, "etc/chariox/app-storage.json")))
    assert.ok(!existsSync(p(h, "usr/libexec/chariox-app-runtime-install")))
    const second = h.install(["alice"])
    assert.equal(second.status, 0, second.out)
    assert.match(second.out, /unchanged/)
    const refused = h.run(["install", "--user", "alice", "--runtime", "retired"])
    assert.notEqual(refused.status, 0)
    assert.match(refused.out, /unknown argument: --runtime/)
  } finally { await rm(h.base, { recursive: true, force: true }) }
})
