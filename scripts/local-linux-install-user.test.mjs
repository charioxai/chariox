import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import { existsSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

// install-user.sh in a fake home, with id, python3 (the enrollment check) and systemctl stubbed.
const installer = fileURLToPath(new URL("../deploy/local-linux/install-user.sh", import.meta.url))

async function harness(t) {
  const base = await mkdtemp(join(tmpdir(), "chariox-install-user-"))
  t.after(() => rm(base, { recursive: true, force: true }))
  const stubs = join(base, "stubs")
  const home = join(base, "home")
  const bundle = join(base, "bundle/bin")
  for (const dir of [stubs, home, bundle]) await mkdir(dir, { recursive: true })
  const stub = async (name, body) => { await writeFile(join(stubs, name), `#!/bin/sh\n${body}\n`); await chmod(join(stubs, name), 0o755) }
  await stub("id", 'case "$1" in -u) echo 1000 ;; -un) echo alice ;; esac')
  await stub("python3", "exit 0")
  await stub("systemctl", `echo "$*" >> "${base}/systemctl.log"; case "$*" in *is-active*|*is-enabled*) [ -e "${base}/active" ] ;; esac; case "$*" in *start*) touch "${base}/active" ;; esac`)
  for (const name of ["chariox-kernel", "chariox-app-wasm-worker"]) {
    await writeFile(join(bundle, name), `#!/bin/sh\necho ${name}\n`)
    await chmod(join(bundle, name), 0o755)
  }
  const run = (...args) => spawnSync("bash", [installer, ...args], {
    encoding: "utf8", env: { PATH: `${stubs}:${process.env.PATH}`, HOME: home, XDG_RUNTIME_DIR: join(base, "run") },
  })
  return { base, home, bundle, run }
}

const mode = async path => (await stat(path)).mode & 0o777

test("the user install puts the WASM App worker beside the installed kernel, read-only for its owner", async t => {
  const { home, bundle, run } = await harness(t)
  const result = run("install", "--kernel", join(bundle, "chariox-kernel"))
  assert.equal(result.status, 0, result.stderr)
  const worker = join(home, ".local/bin/chariox-app-wasm-worker")
  assert.equal(await readFile(worker, "utf8"), "#!/bin/sh\necho chariox-app-wasm-worker\n")
  // The kernel trusts a worker it owns only when the owner cannot write it.
  assert.equal(await mode(worker), 0o555)
  assert.equal(await mode(join(home, ".local/bin/chariox-kernel")), 0o755)
  const again = run("install", "--kernel", join(bundle, "chariox-kernel"))
  assert.equal(again.status, 0, again.stderr)
  assert.match(again.stdout, /unchanged .*chariox-app-wasm-worker/)
  assert.equal(run("uninstall").status, 0)
  assert.equal(existsSync(worker), false)
})

test("the user install refuses a kernel without its worker beside it and changes nothing", async t => {
  const { home, bundle, run } = await harness(t)
  await rm(join(bundle, "chariox-app-wasm-worker"))
  const result = run("install", "--kernel", join(bundle, "chariox-kernel"))
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /chariox-app-wasm-worker is missing/)
  assert.equal(existsSync(join(home, ".local/bin")), false)
})
