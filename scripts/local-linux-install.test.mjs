import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { existsSync, readdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawn, spawnSync } from "node:child_process"
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

async function harness() {
  const base = await mkdtemp(join(tmpdir(), "chariox-local-install-"))
  const root = join(base, "root")
  const bin = join(base, "bin")
  const pkg = join(base, "package")
  const runtime = join(base, "runtime")
  const state = join(base, "state")
  for (const dir of [bin, pkg, runtime, state, join(root, "sys/fs/cgroup"), join(root, "sys/module/apparmor/parameters"),
    join(root, "sys/kernel/security/apparmor"), join(root, "var/lib/systemd/linger")]) {
    await mkdir(dir, { recursive: true })
  }
  await writeFile(join(root, "sys/fs/cgroup/cgroup.controllers"), "cpu memory pids\n")
  await writeFile(join(root, "sys/module/apparmor/parameters/enabled"), "Y\n")
  await writeFile(join(root, "sys/kernel/security/apparmor/profiles"), "")
  const passwd = Object.entries(users).map(([name, uid]) => `${name}:x:${uid}:${groups[name]}::/home/${name}:/bin/bash`)
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
  "show user@"*) echo "\${HARNESS_DELEGATE:-cpu memory pids}" ;;
  "is-enabled --quiet chariox-app-storage.service") test -e "$HARNESS_STATE/enabled" ;;
  "is-active --quiet chariox-app-storage.service") test -e "$HARNESS_STATE/active" ;;
  "enable --quiet chariox-app-storage.service") touch "$HARNESS_STATE/enabled" ;;
  "start chariox-app-storage.service"|"restart chariox-app-storage.service") touch "$HARNESS_STATE/active" ;;
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
case "$1" in -r) printf "chariox-app-bwrap (unconfined)\\nchariox-app-domain-entry (unconfined)\\n" > "$profiles" ;; -R) : > "$profiles" ;; esac
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
  await script(join(pkg, "chariox-app-storage"), "#!/bin/sh\necho helper v1\n")
  // Stands in for the repository's root runtime installer: it verifies the
  // trusted key and digest and publishes the enrollment it would write.
  await script(join(pkg, "chariox-app-runtime-install"), `#!/bin/sh
echo "$*" >> "$HARNESS_STATE/runtime-install"
[ "$1" = install ] && [ "$5" = ${KEY} ] || { echo "signature verification failed" >&2; exit 1; }
mkdir -p "$CHARIOX_LOCAL_INSTALL_ROOT/etc/chariox/apps" "$CHARIOX_LOCAL_INSTALL_ROOT/usr/lib/chariox/app-runtimes/$7"
touch "$CHARIOX_LOCAL_INSTALL_ROOT/usr/lib/chariox/app-runtimes/$7/.runtime-lease"
printf '{"revision":1,"inventorySha256":"%s"}\\n' "$7" > "$CHARIOX_LOCAL_INSTALL_ROOT/etc/chariox/apps/runtime-enrollment.json"
`)
  const inventory = '{"schema":"chariox.app-runtime-inventory.v1"}\n'
  await writeFile(join(runtime, "runtime-inventory.json"), inventory)
  await writeFile(join(runtime, "runtime-inventory.sig"), "b".repeat(128))
  const digest = createHash("sha256").update(inventory).digest("hex")
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, CHARIOX_LOCAL_INSTALL_ROOT: root, HARNESS_STATE: state }
  const run = (args, extra = {}) => {
    const result = spawnSync("bash", [installer, ...args], { env: { ...env, ...extra }, encoding: "utf8" })
    return { ...result, out: result.stdout + result.stderr }
  }
  const install = (who, extra = [], env = {}) =>
    run(["install", ...who.flatMap((name) => ["--user", name]), "--bin", pkg, "--runtime", runtime,
      "--runtime-key", KEY, "--runtime-digest", digest, ...extra], env)
  const log = async (name) => (existsSync(join(state, name)) ? readFile(join(state, name), "utf8") : "")
  const reset = (name) => rm(join(state, name), { force: true })
  return { base, root, pkg, runtime, digest, run, install, log, reset }
}

const p = (h, path) => join(h.root, path)
const owners = async (h) => JSON.parse(await readFile(p(h, "etc/chariox/app-storage.json"), "utf8")).owners
const cgroup = (uid) => `/sys/fs/cgroup/user.slice/user-${uid}.slice/user@${uid}.service/app.slice/chariox-kernel.service/apps`
const created = (h) => ["etc/chariox", "usr/lib/chariox", "usr/libexec/chariox-app-storage", "usr/libexec/chariox-app-runtime-install",
  "etc/systemd/system/chariox-app-storage.service", "etc/apparmor.d/chariox-app-bwrap", "var/lib/chariox-app-storage",
  "var/lib/chariox-local-install"].filter((path) => existsSync(p(h, path)))

test("local Linux root install refuses a non-root caller and bad input without changing anything", { skip: !linux }, async () => {
  const h = await harness()
  try {
    const notRoot = h.install(["alice"], [], { HARNESS_UID: "1000" })
    assert.equal(notRoot.status, 1)
    assert.match(notRoot.out, /run this as root/)
    const wrongDigest = h.run(["install", "--user", "alice", "--bin", h.pkg, "--runtime", h.runtime,
      "--runtime-key", KEY, "--runtime-digest", "c".repeat(64)])
    assert.equal(wrongDigest.status, 1)
    assert.match(wrongDigest.out, /runtime-inventory\.json has SHA-256 [0-9a-f]{64}, not the expected c{64}/)
    const wrongKey = h.run(["install", "--user", "alice", "--bin", h.pkg, "--runtime", h.runtime,
      "--runtime-key", "d".repeat(64), "--runtime-digest", h.digest])
    assert.equal(wrongKey.status, 1)
    assert.match(wrongKey.out, /signature verification failed/)
    const root = h.install(["root"])
    assert.equal(root.status, 1)
    assert.match(root.out, /must run as an ordinary user/)
    const path = h.install(["../alice"])
    assert.equal(path.status, 1)
    assert.match(path.out, /not a user name: \.\.\/alice/)
    const undelegated = h.install(["alice"], [], { HARNESS_DELEGATE: "memory pids" })
    assert.equal(undelegated.status, 1)
    assert.match(undelegated.out, /does not delegate the cpu controller/)
    assert.deepEqual(created(h), [])
    assert.equal(await h.log("systemctl").then((log) => /start|restart|enable/.test(log)), false)
    const dryRun = h.install(["alice"], ["--dry-run"])
    assert.equal(dryRun.status, 0, dryRun.out)
    assert.match(dryRun.out, /would verify and install the signed App runtime/)
    assert.match(dryRun.out, /would write .*app-storage\.json with 1 owner/)
    assert.match(dryRun.out, /would enable lingering for alice/)
    // Any existing account in the POSIX portable name set, not only lowercase names.
    const portable = h.install(["Carol.Smith"], ["--dry-run"])
    assert.equal(portable.status, 0, portable.out)
    assert.match(portable.out, /would enable lingering for Carol\.Smith/)
    assert.deepEqual(created(h), [])
    // Only the refused key ever reached the runtime installer; the dry run did not.
    assert.equal(await h.log("runtime-install"), `install --source ${h.runtime} --trusted-public-key-hex ${"d".repeat(64)} --inventory-sha256 ${h.digest}\n`)
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})

test("local Linux root install is idempotent, merges owners and uninstalls", { skip: !linux }, async () => {
  const h = await harness()
  try {
    const first = h.install(["alice"])
    assert.equal(first.status, 0, first.out)
    for (const line of [
      /verify and install the signed App runtime [0-9a-f]{64} from /,
      /enrolled runtime [0-9a-f]{64} revision 1 \(was none\)/,
      /install .*\/usr\/libexec\/chariox-app-storage/,
      /install .*\/etc\/systemd\/system\/chariox-app-storage\.service/,
      /write .*\/etc\/chariox\/app-storage\.json with 1 owner/,
      /load AppArmor profile chariox-app-bwrap/,
      /enable lingering for alice/,
      /start chariox-app-storage\.service/,
    ]) {
      assert.match(first.out, line)
    }
    assert.equal(await h.log("runtime-install"), `install --source ${h.runtime} --trusted-public-key-hex ${KEY} --inventory-sha256 ${h.digest}\n`)
    assert.deepEqual(await owners(h), [{ uid: 1000, gid: 1000, cgroup_root: cgroup(1000), kernel_database_paths: ["/home/alice/.chariox/state/kernel.db"] }])
    assert.equal(await readFile(p(h, "etc/apparmor.d/chariox-app-bwrap"), "utf8"),
      await readFile(join(repositoryRoot, "deploy/local-linux/chariox-app-bwrap.apparmor"), "utf8"))
    assert.equal(await readFile(p(h, "etc/systemd/system/chariox-app-storage.service"), "utf8"),
      await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-app-storage.service"), "utf8"))
    assert.ok(existsSync(p(h, "var/lib/chariox-local-install/linger-1000")))

    await h.reset("systemctl")
    const again = h.install(["alice"])
    assert.equal(again.status, 0, again.out)
    assert.doesNotMatch(again.out, /\] (install|write|load|enable|start|restart) /)
    assert.match(again.out, /unchanged runtime [0-9a-f]{64} revision 1 \(re-verified\)/)
    assert.match(again.out, /unchanged .*app-storage\.json/)
    assert.match(again.out, /unchanged chariox-app-storage\.service \(running\)/)
    assert.doesNotMatch(await h.log("systemctl"), /restart|start /)

    // A lost native-entry profile must be repaired even when the file and
    // existing bwrap profile are unchanged. No helper restart is required.
    await writeFile(p(h, "sys/kernel/security/apparmor/profiles"), "chariox-app-bwrap (unconfined)\n")
    await h.reset("apparmor")
    const repairedProfile = h.install(["alice"])
    assert.equal(repairedProfile.status, 0, repairedProfile.out)
    assert.match(await h.log("apparmor"), /^-r /m)
    assert.doesNotMatch(await h.log("systemctl"), /restart|start /)

    const second = h.install(["bob"])
    assert.equal(second.status, 0, second.out)
    assert.match(second.out, /write .*app-storage\.json with 2 owner/)
    assert.match(await h.log("systemctl"), /^restart chariox-app-storage\.service$/m)
    assert.deepEqual((await owners(h)).map((owner) => owner.uid), [1000, 1001])

    await mkdir(p(h, "var/lib/chariox-app-storage/u-1000/i-0"), { recursive: true })
    const busy = h.run(["uninstall", "--user", "alice"])
    assert.equal(busy.status, 1)
    assert.match(busy.out, /alice still has App storage/)
    assert.deepEqual((await owners(h)).map((owner) => owner.uid), [1000, 1001])
    await rm(p(h, "var/lib/chariox-app-storage/u-1000/i-0"), { recursive: true })
    const kernel = p(h, "sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/chariox-kernel.service")
    await mkdir(kernel, { recursive: true })
    const running = h.run(["uninstall", "--user", "alice"])
    assert.equal(running.status, 1)
    assert.match(running.out, /alice's kernel is running; run install-user\.sh uninstall as alice first/)
    await rm(join(h.root, "sys/fs/cgroup/user.slice"), { recursive: true })
    await h.reset("systemctl")
    const alice = h.run(["uninstall", "--user", "alice"])
    assert.equal(alice.status, 0, alice.out)
    assert.deepEqual((await owners(h)).map((owner) => owner.uid), [1001])
    assert.match(await h.log("loginctl"), /^disable-linger alice$/m)
    assert.match(await h.log("systemctl"), /^restart chariox-app-storage\.service$/m)
    assert.equal(existsSync(p(h, "var/lib/systemd/linger/alice")), false)
    // The helper refuses to start with a storage directory of an unenrolled owner.
    assert.equal(existsSync(p(h, "var/lib/chariox-app-storage/u-1000")), false)
    const removed = alice.out.indexOf(`remove ${p(h, "var/lib/chariox-app-storage/u-1000")} (empty)`)
    assert.ok(removed >= 0 && removed < alice.out.indexOf("write "), alice.out)

    // A kernel holding a runtime generation's shared lease blocks --all before any change.
    const holder = spawn("flock", ["-s", "-o", p(h, `usr/lib/chariox/app-runtimes/${h.digest}/.runtime-lease`), "sh", "-c", "echo held; exec sleep 30"],
      { detached: true })
    await once(holder.stdout, "data")
    const inUse = h.run(["uninstall", "--all"])
    process.kill(-holder.pid)
    await once(holder, "exit")
    assert.equal(inUse.status, 1)
    assert.match(inUse.out, /runtime [0-9a-f]{64} is in use/)
    assert.ok(existsSync(p(h, "etc/chariox/apps/runtime-enrollment.json")))
    const all = h.run(["uninstall", "--all"])
    assert.equal(all.status, 0, all.out)
    assert.match(await h.log("loginctl"), /^disable-linger bob$/m)
    assert.match(await h.log("apparmor"), /^-R /m)
    assert.deepEqual(created(h), [])
    assert.deepEqual(readdirSync(p(h, "var/lib")), ["systemd"])
  } finally {
    await rm(h.base, { recursive: true, force: true })
  }
})

test("local Linux kernel unit owns the delegated subtree the root install enrolls", async () => {
  const unit = await readFile(join(repositoryRoot, "deploy/local-linux/chariox-kernel.service"), "utf8")
  const start = await readFile(join(repositoryRoot, "deploy/local-linux/start-kernel.sh"), "utf8")
  const root = await readFile(installer, "utf8")
  for (const line of ["Delegate=cpu memory pids", "DelegateSubgroup=supervisor", "Environment=CHARIOX_HOME=%h/.chariox",
    "ExecStart=%h/.local/lib/chariox/start-kernel.sh %h/.local/bin/chariox-kernel"]) {
    assert.ok(unit.split("\n").includes(line), `chariox-kernel.service is missing ${line}`)
  }
  // The domain is ready before the kernel runs (it starts enabled Apps at once),
  // prepared from the supervisor subgroup: an ExecStartPre enabling controllers
  // makes systemd 259 fail the main process spawn, and a Post races the kernel.
  assert.doesNotMatch(unit, /^ExecStart(Pre|Post)=/m)
  assert.match(start, /chariox-kernel\.service\/supervisor \]\]/)
  assert.ok(start.indexOf('mkdir "$unit/apps"') < start.indexOf('exec "$1"'))
  assert.match(root, /user@\$uid\.service\/app\.slice\/chariox-kernel\.service\/apps:\$home\/\.chariox\/state\/kernel\.db/)
})

test("local Linux root install refuses GID zero before changing shared enrollment", { skip: !linux }, async () => {
  const h = await harness()
  try {
    assert.equal(h.install(["alice"]).status, 0)
    const enrollment = await readFile(p(h, "etc/chariox/app-storage.json"), "utf8")
    const logs = Object.fromEntries(await Promise.all(
      ["runtime-install", "systemctl", "loginctl", "apparmor"].map(async (name) => [name, await h.log(name)]),
    ))
    const result = h.install(["bob", "rootgroup"])
    assert.equal(result.status, 1)
    assert.match(result.out, /rootgroup must have a non-root primary group/)
    assert.equal(await readFile(p(h, "etc/chariox/app-storage.json"), "utf8"), enrollment)
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
  const result = spawnSync("apparmor_parser", ["-Q", join(repositoryRoot, "deploy/local-linux/chariox-app-bwrap.apparmor")], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr || result.stdout)
})
