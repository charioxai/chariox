import assert from "node:assert/strict"
import { test } from "node:test"
import { chmod, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "@chariox/kernel-client"
import { locateAppPackageBinary, runAppDeveloperCommand, type AppDeveloperDeps } from "./app-developer.js"
import { runAppCommand } from "./app-command.js"

function harness(response = { status: 0, stdout: '{"ok":true,"result":{"status":"fixture"}}' }) {
  const calls: { binary: string, args: readonly string[] }[] = []
  const output: string[] = []
  const deps: AppDeveloperDeps = {
    locateBinary: async () => "/installed/chariox-app-package",
    execute: async (binary, args) => { calls.push({ binary, args }); return response },
    write: (text) => { output.push(text) },
  }
  return { calls, output, deps }
}

test("developer commands use one local package tool with current protocol and no kernel", async () => {
  for (const action of ["create", "pack", "validate", "manifest", "inspect", "keygen"]) {
    const h = harness()
    assert.equal(await runAppCommand(["app", action, "space ; $(touch no-file)"], {
      createClient: () => { throw new Error("must not connect") }, write: () => { throw new Error("wrong output path") }, developer: h.deps,
    }), true)
    assert.deepEqual(h.calls[0], {
      binary: "/installed/chariox-app-package",
      args: [action, "space ; $(touch no-file)", ...(["create", "pack", "validate", "manifest"].includes(action) ? ["--kernel-protocol", String(LOCAL_DAEMON_PROTOCOL_VERSION)] : [])],
    })
    assert.equal(JSON.parse(h.output[0]!).status, "fixture")
  }
})

test("developer argument and response failures are bounded and do not claim success", async () => {
  const h = harness()
  await assert.rejects(runAppDeveloperCommand(["create", "--kernel-protocol", "1"], h.deps), /supplies its current protocol/)
  await assert.rejects(runAppDeveloperCommand(["create", "x".repeat(4097)], h.deps), /bounds/)
  assert.equal(h.calls.length, 0)
  assert.equal(await runAppDeveloperCommand(["list"], h.deps), false)
  for (const response of [
    { status: 2, stdout: '{"ok":false,"error":{"code":"INVALID_SIGNATURE","message":"signature rejected"}}' },
    { status: 0, stdout: "not json" }, { status: 1, stdout: '{"ok":true,"result":{}}' },
  ]) {
    const failed = harness(response)
    await assert.rejects(runAppDeveloperCommand(["inspect", "sample.cxapp"], failed.deps))
    assert.deepEqual(failed.output, [])
  }
})

test("an explicit helper must be absolute and executable; it never falls back", async (context) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "chariox-app-helper-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const binary = join(root, "chariox-app-package")
  await writeFile(binary, "#!/bin/sh\nexit 0\n", { mode: 0o600 })
  await assert.rejects(locateAppPackageBinary({ CHARIOX_APP_PACKAGE_BIN: "relative-helper" }), /absolute/)
  await assert.rejects(locateAppPackageBinary({ CHARIOX_APP_PACKAGE_BIN: binary }), /not an executable/)
  await chmod(binary, 0o755)
  assert.equal(await locateAppPackageBinary({ CHARIOX_APP_PACKAGE_BIN: binary }), binary)
  await assert.rejects(locateAppPackageBinary({ CHARIOX_APP_PACKAGE_BIN: join(root, "missing") }), /not an executable/)
})

test("source helper discovery uses module checkout and explicit target directory", async (context) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "chariox-app-source-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(join(root, "packages/app-package"), { recursive: true })
  await writeFile(join(root, "packages/app-package/Cargo.toml"), "[package]\n")
  const target = join(root, "external-build")
  await mkdir(join(target, "debug"), { recursive: true })
  const binary = join(target, "debug/chariox-app-package")
  await writeFile(binary, "#!/bin/sh\nexit 0\n", { mode: 0o755 })
  // An explicitly selected checkout target takes precedence over installed tools.
  assert.equal(await locateAppPackageBinary({ CARGO_TARGET_DIR: target }, pathToFileURL(join(root, "apps/cli/dist/app-developer.js")).href), binary)
})
