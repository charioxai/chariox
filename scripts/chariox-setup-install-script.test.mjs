// MP-07 / MP-11: bootstrap must verify Setup before executing it, without ticket argv.
import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, mkdir, mkdtemp, readFile, realpath, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { promisify } from "node:util"
import test from "node:test"
import { renderInstallScript } from "./build-chariox-setup.mjs"
const execute = promisify(execFile), source = await readFile(new URL("../deploy/setup/install.sh", import.meta.url), "utf8")
async function harness(t) {
  const root = await mkdtemp(join(await realpath(tmpdir()), "chariox-setup-script-")); t.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin"), payload = join(root, "payload"), sig = join(root, "sig"), script = join(root, "install.sh"), log = join(root, "executed")
  await mkdir(bin)
  const keys = generateKeyPairSync("ed25519"), publicKeyHex = keys.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const bytes = Buffer.from('#!/bin/sh\nprintf "%s\\n" "$*" > "$SETUP_FIXTURE_LOG"\nif [ "${3:-}" = "--enroll" ]; then IFS= read -r code; unset code; fi\n')
  await writeFile(payload, bytes); await writeFile(sig, sign(null, bytes, keys.privateKey).toString("hex"))
  await writeFile(script, renderInstallScript(source, { version: "0.3.0", publicKeyHex, releaseBase: "https://releases.example.test" }))
  await writeFile(join(bin, "curl"), `#!/bin/sh\noutput=\nsource="$SETUP_FIXTURE_PAYLOAD"\nfor arg do\n  if [ "$previous" = -o ]; then output=$arg; fi\n  case "$arg" in *.sig) source="$SETUP_FIXTURE_SIG" ;; esac\n  previous=$arg\ndone\ncp "$source" "$output"\n`, { mode: 0o755 })
  // Linux bootstrap contract; macOS platform detection is tested separately in Setup.
  await writeFile(join(bin, "uname"), '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; *) exit 1;; esac\n', { mode: 0o755 })
  const env = { PATH: `${bin}:${process.env.PATH}`, HOME: root, TMPDIR: root, SETUP_FIXTURE_LOG: log, SETUP_FIXTURE_PAYLOAD: payload, SETUP_FIXTURE_SIG: sig }
  const run = async (args = [], input) => {
    const child = execFile("sh", [script, ...args], { env, timeout: 15_000 })
    if (input) { child.stdin.on("error", () => {}); child.stdin.end(input) }
    return new Promise((yes, no) => { child.once("error", no); child.once("close", code => yes(code)) })
  }
  return { root, payload, sig, log, env, script, run }
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
