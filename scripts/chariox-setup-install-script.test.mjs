// MP-07 / MP-11: bootstrap must verify Setup before executing it, without ticket argv.
import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { generateKeyPairSync, sign } from "node:crypto"
import { mkdir, mkdtemp, readFile, realpath, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { promisify } from "node:util"
import test from "node:test"
import { renderInstallScript } from "./build-chariox-setup.mjs"
const execute = promisify(execFile), source = await readFile(new URL("../deploy/setup/install.sh", import.meta.url), "utf8")
async function harness(t, { platform = "Linux", stockLibreSSL = false } = {}) {
  const root = await mkdtemp(join(await realpath(tmpdir()), "chariox-setup-script-")); t.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin"), payload = join(root, "payload"), sig = join(root, "sig"), script = join(root, "install.sh"), log = join(root, "executed"), downloads = join(root, "downloads"), verifier = join(root, "verifier")
  await mkdir(bin)
  const keys = generateKeyPairSync("ed25519"), publicKeyHex = keys.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const bytes = Buffer.from('#!/bin/sh\nprintf "%s\\n" "$*" > "$SETUP_FIXTURE_LOG"\nif [ "${3:-}" = "--enroll" ]; then IFS= read -r code; unset code; fi\n')
  await writeFile(payload, bytes); await writeFile(sig, sign(null, bytes, keys.privateKey).toString("hex"))
  await writeFile(script, renderInstallScript(source, { version: "0.3.0", publicKeyHex, releaseBase: "https://releases.example.test" }))
  await writeFile(join(bin, "curl"), `#!/bin/sh\nprintf '%s\\n' "$@" >> "$SETUP_FIXTURE_DOWNLOAD_LOG"\noutput=\nsource="$SETUP_FIXTURE_PAYLOAD"\nfor arg do\n  if [ "$previous" = -o ]; then output=$arg; fi\n  case "$arg" in *.sig) source="$SETUP_FIXTURE_SIG" ;; esac\n  previous=$arg\ndone\ncp "$source" "$output"\n`, { mode: 0o755 })
  await writeFile(join(bin, "uname"), `#!/bin/sh\ncase "$1" in -s) echo ${platform};; -m) echo ${platform === "Darwin" ? "arm64" : "x86_64"};; *) exit 1;; esac\n`, { mode: 0o755 })
  if (stockLibreSSL) {
    // MP-11: model the stock macOS version and upstream pkeyutl parser's -rawin refusal.
    await writeFile(join(bin, "openssl"), `#!/bin/sh
printf '%s\\n' "$*" >> "$SETUP_FIXTURE_VERIFIER_LOG"
case "$1" in
  version) echo 'LibreSSL 3.3.6'; exit 0 ;;
  pkeyutl)
    for arg do
      if [ "$arg" = -rawin ]; then echo 'unknown option: -rawin' >&2; exit 1; fi
    done ;;
esac
exit 1
`, { mode: 0o755 })
  }
  const env = { PATH: `${bin}:${process.env.PATH}`, HOME: root, TMPDIR: root, SETUP_FIXTURE_LOG: log, SETUP_FIXTURE_PAYLOAD: payload, SETUP_FIXTURE_SIG: sig, SETUP_FIXTURE_DOWNLOAD_LOG: downloads, SETUP_FIXTURE_VERIFIER_LOG: verifier }
  const run = async (args = [], input) => {
    const child = execFile("sh", [script, ...args], { env, timeout: 15_000 })
    if (input) { child.stdin.on("error", () => {}); child.stdin.end(input) }
    return new Promise((yes, no) => { child.once("error", no); child.once("close", code => yes(code)) })
  }
  return { root, payload, sig, log, downloads, verifier, env, script, run }
}
for (const valid of [true, false]) {
  test(`MP-07/MP-11 Darwin stock LibreSSL reports its prerequisite before downloads (${valid ? "valid" : "invalid"} signature)`, async t => {
    const h = await harness(t, { platform: "Darwin", stockLibreSSL: true })
    if (!valid) await writeFile(h.sig, "b".repeat(128))
    const result = await execute("sh", [h.script], { env: h.env, timeout: 15_000 }).catch(error => error)
    assert.equal(result.code, 1)
    assert.match(result.stderr, /requires OpenSSL 3/)
    assert.match(result.stderr, /LibreSSL/)
    assert.match(result.stderr, /brew install openssl@3/)
    assert.match(result.stderr, /PATH/)
    assert.doesNotMatch(result.stderr, /Setup signature refused/)
    assert.equal(await readFile(h.verifier, "utf8"), "version\n")
    await assert.rejects(readFile(h.downloads))
    await assert.rejects(readFile(h.log))
    assert.equal((await readdir(h.root)).some(name => name.startsWith("chariox-setup.")), false)
  })
  test(`MP-07/MP-11 Darwin compatible OpenSSL ${valid ? "executes valid" : "refuses invalid"} signed Setup and cleans scratch`, async t => {
    const h = await harness(t, { platform: "Darwin" })
    if (!valid) await writeFile(h.sig, "b".repeat(128))
    const result = await execute("sh", [h.script], { env: h.env, timeout: 15_000 }).catch(error => error)
    assert.equal(result.code ?? 0, valid ? 0 : 1)
    assert.match(await readFile(h.downloads, "utf8"), /chariox-setup-0\.3\.0-darwin-arm64/)
    if (valid) assert.equal(await readFile(h.log, "utf8"), "--install-only --login\n")
    else {
      assert.match(result.stderr, /Setup signature refused/)
      await assert.rejects(readFile(h.log))
    }
    assert.equal((await readdir(h.root)).some(name => name.startsWith("chariox-setup.")), false)
  })
}
test("MP-07 script verifies public pin then runs CLI login, and removes its scratch", async t => {
  const h = await harness(t)
  assert.equal(await h.run(), 0)
  assert.equal(await readFile(h.log, "utf8"), "--install-only --login\n")
  assert.equal((await readdir(h.root)).some(name => name.startsWith("chariox-setup.")), false)
})
test("MP-11 code stays on stdin; argv codes are refused before downloads/execution", async t => {
  const h = await harness(t)
  assert.equal(await h.run(["--enroll"], "synthetic-one-use-code\n"), 0)
  assert.equal(await readFile(h.log, "utf8"), "--install-only --login --enroll\n")
  await rm(h.log)
  assert.equal(await h.run(["--enroll", "must-not-be-an-argument"]), 1)
  await assert.rejects(readFile(h.log))
})
test("MP-07 corrupted Setup or signature never executes the payload", async t => {
  for (const field of ["payload", "sig"]) {
    const h = await harness(t)
    await writeFile(h[field], field === "payload" ? "#!/bin/sh\nexit 0\n" : "b".repeat(128))
    assert.equal(await h.run(), 1)
    await assert.rejects(readFile(h.log))
  }
})
test("MP-07 public build inputs reject shell syntax and missing release authority", () => {
  for (const patch of [{ version: "1;id" }, { publicKeyHex: "" }, { releaseBase: "https://x/'$(id)" }]) assert.throws(() => renderInstallScript(source, { version: "0.3.0", publicKeyHex: "a".repeat(64), releaseBase: "https://releases.example", ...patch }))
})
