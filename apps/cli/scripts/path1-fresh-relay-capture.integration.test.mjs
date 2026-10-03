import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { createRequire } from "node:module"
import { test } from "node:test"
import { access, cp, copyFile, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, relative, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")
const dependencyRoot = resolve(process.env.PATH1_FRESH_RELAY_TEST_DEPENDENCY_WORKTREE || sourceRoot)
const dependencyRequire = createRequire(join(dependencyRoot, "packages/kernel-client/package.json"))
const FIXTURE_SECRET = "FIXTURE_PROVIDER_PAYLOAD_SECRET_MUST_NOT_LEAK"
const CLIENT_TIMEOUT_MS = 24_000
let buildRoot
let runtimeRoot
let runtimeEntry

function runBuild(command, args, cwd, env) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: "utf8",
    maxBuffer: 2 * 1024 * 1024,
    timeout: 120_000,
  })
  assert.equal(result.error, undefined, result.error?.message)
  assert.equal(result.status, 0, (result.stdout || "") + (result.stderr || ""))
}

async function copyPackage(name) {
  const source = join(sourceRoot, name)
  const target = join(runtimeRoot, name)
  await mkdir(target, { recursive: true })
  await cp(join(source, "src"), join(target, "src"), { recursive: true })
  await copyFile(join(source, "package.json"), join(target, "package.json"))
  await copyFile(join(source, "tsconfig.json"), join(target, "tsconfig.json"))
}

async function buildRuntime() {
  buildRoot = await mkdtemp(join(tmpdir(), "chariox-path1-fresh-relay-build-"))
  runtimeRoot = join(buildRoot, "workspace")
  await mkdir(runtimeRoot)
  const buildEnv = {
    PATH: process.env.PATH || "/usr/bin:/bin",
    TMPDIR: buildRoot,
  }

  await copyFile(join(sourceRoot, "tsconfig.base.json"), join(runtimeRoot, "tsconfig.base.json"))
  await symlink(join(dependencyRoot, "node_modules"), join(runtimeRoot, "node_modules"), "dir")
  await copyPackage("packages/tool-display")
  await copyPackage("packages/kernel-client")
  await cp(
    join(dependencyRoot, "packages/kernel-client/node_modules"),
    join(runtimeRoot, "packages/kernel-client/node_modules"),
    { recursive: true },
  )

  const cliRoot = join(runtimeRoot, "apps/cli")
  await mkdir(join(cliRoot, "src"), { recursive: true })
  await mkdir(join(cliRoot, "scripts"), { recursive: true })
  await copyFile(join(sourceRoot, "apps/cli/package.json"), join(cliRoot, "package.json"))
  await copyFile(
    join(sourceRoot, "apps/cli/src/protocol-minimum-diagnostic.ts"),
    join(cliRoot, "src/protocol-minimum-diagnostic.ts"),
  )
  for (const name of [
    "build.mjs",
    "path1-fresh-relay-capture.mjs",
    "path1-provider-rebuild-capture.mjs",
  ]) {
    await copyFile(join(sourceRoot, "apps/cli/scripts", name), join(cliRoot, "scripts", name))
  }
  await symlink(join(dependencyRoot, "apps/cli/node_modules"), join(cliRoot, "node_modules"), "dir")

  const tsc = join(dependencyRoot, "node_modules/typescript/bin/tsc")
  await access(tsc)
  runBuild(process.execPath, [tsc, "-p", join(runtimeRoot, "packages/tool-display/tsconfig.json")], runtimeRoot, buildEnv)
  runBuild(process.execPath, [tsc, "-p", join(runtimeRoot, "packages/kernel-client/tsconfig.json")], runtimeRoot, buildEnv)
  runBuild(process.execPath, [join(cliRoot, "scripts/build.mjs")], runtimeRoot, buildEnv)

  runtimeEntry = join(cliRoot, "scripts/path1-fresh-relay-capture.mjs")
  await access(join(runtimeRoot, "packages/kernel-client/dist/ipc.js"))
  await access(join(cliRoot, "dist/protocol-minimum-diagnostic.js"))
}

function startFakeKernel(mode) {
  const { WebSocketServer } = dependencyRequire("ws")
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0, maxPayload: 1024 * 1024 })
  const requests = []
  server.on("connection", (socket) => {
    socket.on("message", (message) => {
      const request = JSON.parse(message.toString())
      requests.push(request)
      if (mode === "timeout") return

      const now = Date.now()
      const wrongScope = mode === "wrong-scope"
      const response = {
        FreshRemoteMachineKernelsObserved: {
          machine_ref: "machine-resolved-346",
          query_started_at_ms: now - 35,
          query_completed_at_ms: now - 8,
          kernels: [{
            kernel_id: "kernel-registration-346",
            machine_id: wrongScope ? "machine-other" : "machine-resolved-346",
            machine_alias: wrongScope ? null : "builder-west",
            relay_alias: null,
            kernel_alias: "worker-blue",
            available_providers: [FIXTURE_SECRET],
            provider_accounts: [{ token: FIXTURE_SECRET }],
            capabilities: [FIXTURE_SECRET],
            public_key: FIXTURE_SECRET,
          }],
        },
      }
      const error = mode === "old-kernel"
        ? { code: "invalid_request", message: 'unknown variant "QueryFreshRemoteMachineKernels"', retryable: false }
        : mode === "error"
          ? { code: "fixture_error", message: FIXTURE_SECRET, retryable: false }
          : null
      socket.send(JSON.stringify({
        type: "response",
        request_id: request.request_id,
        response: error ? null : response,
        error,
      }))
    })
  })
  return new Promise((resolve, reject) => {
    server.once("error", reject)
    server.once("listening", () => {
      const address = server.address()
      assert.ok(address && typeof address === "object")
      resolve({
        endpoint: "ws://127.0.0.1:" + address.port,
        requests,
        async close() {
          for (const socket of server.clients) socket.terminate()
          await new Promise((done) => server.close(done))
        },
      })
    })
  })
}

function runCli(endpoint, outputPath, entryPath = runtimeEntry) {
  const child = spawn(process.execPath, [
    entryPath,
    "--kernel", endpoint,
    "--machine", "builder-west",
    "--output", outputPath,
  ], {
    cwd: runtimeRoot,
    env: {
      PATH: process.env.PATH || "/usr/bin:/bin",
      TMPDIR: buildRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  let timedOut = false
  const timer = setTimeout(() => {
    timedOut = true
    child.kill("SIGKILL")
  }, CLIENT_TIMEOUT_MS)
  child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk })
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk })
  return new Promise((resolve, reject) => {
    child.once("error", reject)
    child.once("close", (code, signal) => {
      clearTimeout(timer)
      resolve({ code, signal, stdout, stderr, timedOut })
    })
  })
}

function assertFreshRequests(requests, expectSingle, result) {
  assert.ok(requests.length > 0,
    "CLI must issue the fresh relay request; exit=" + result.code + " stdout=" + result.stdout + " stderr=" + result.stderr)
  if (expectSingle) assert.equal(requests.length, 1)
  for (const request of requests) {
    assert.equal(request.type, "request")
    assert.match(request.request_id, /^[0-9a-f-]{36}$/)
    assert.equal(request.command_id, request.request_id)
    assert.deepEqual(request.request, {
      QueryFreshRemoteMachineKernels: { machine_ref: "builder-west" },
    })
  }
}

async function exercise(mode, check, throughSymlink = false) {
  const createdEvidenceRoot = await mkdtemp(join(tmpdir(), "chariox-path1-fresh-relay-evidence-"))
  let evidenceRoot = createdEvidenceRoot
  const entryAlias = throughSymlink ? join(buildRoot, "path1-fresh-relay-capture-alias.mjs") : null
  let kernel
  try {
    evidenceRoot = await realpath(createdEvidenceRoot)
    assert.equal(await realpath(evidenceRoot), evidenceRoot, "fixture output directory must be canonical")
    const outputPath = join(evidenceRoot, "capture.json")
    if (entryAlias) await symlink(runtimeEntry, entryAlias)
    kernel = await startFakeKernel(mode)
    const result = await runCli(kernel.endpoint, outputPath, entryAlias || runtimeEntry)
    assertFreshRequests(kernel.requests, mode !== "timeout", result)
    assert.equal(result.timedOut, false, "CLI process must remain bounded")
    assert.ok(!result.stdout.includes(FIXTURE_SECRET))
    assert.ok(!result.stderr.includes(FIXTURE_SECRET))
    await check({ result, outputPath, evidenceRoot })
  } finally {
    if (kernel) await kernel.close()
    if (entryAlias) await rm(entryAlias, { force: true })
    await rm(createdEvidenceRoot, { recursive: true, force: true })
  }
}

async function assertNoEvidence(path) {
  await assert.rejects(stat(path), (error) => error.code === "ENOENT")
}

test("Path-1 fresh relay production CLI integration", { timeout: 120_000 }, async (suite) => {
  try {
    await buildRuntime()

await suite.test("real CLI captures protocol 346 observations to a private external file", async () => {
  await exercise("valid", async ({ result, outputPath, evidenceRoot }) => {
    assert.equal(result.code, 0)
    assert.equal(result.stderr, "")
    assert.match(result.stdout, /current scoped relay registration observation/)
    assert.equal(await realpath(dirname(outputPath)), await realpath(evidenceRoot))
    assert.equal(relative(runtimeRoot, await realpath(outputPath)).startsWith(".." + sep), true)
    assert.equal((await stat(outputPath)).mode & 0o777, 0o600)
    const capture = JSON.parse(await readFile(outputPath, "utf8"))
    assert.equal(capture.minimumProtocolVersion, 346)
    assert.equal(capture.scope.requestedMachineRef, "builder-west")
    assert.equal(capture.scope.resolvedMachineRef, "machine-resolved-346")
    assert.equal(capture.kernels.length, 1)
    assert.equal(capture.kernels[0].kernelId, "kernel-registration-346")
    assert.equal(capture.verdict, undefined)
    assert.ok(!JSON.stringify(capture).includes(FIXTURE_SECRET))
  })
})

await suite.test("real CLI entrypoint works through a filesystem symlink", async () => {
  await exercise("valid", async ({ result, outputPath }) => {
    assert.equal(result.code, 0)
    assert.equal(result.stderr, "")
    assert.match(result.stdout, /current scoped relay registration observation/)
    assert.equal((await stat(outputPath)).mode & 0o777, 0o600)
    const capture = JSON.parse(await readFile(outputPath, "utf8"))
    assert.equal(capture.minimumProtocolVersion, 346)
  }, true)
})

await suite.test("real CLI reports the older-kernel protocol minimum without writing evidence", async () => {
  await exercise("old-kernel", async ({ result, outputPath }) => {
    assert.equal(result.code, 1)
    assert.equal(result.stderr, "Home kernel requires local daemon protocol 346 or newer.\n")
    assert.equal(result.stdout, "")
    await assertNoEvidence(outputPath)
  })
})

await suite.test("real CLI rejects wrong-scope registrations without writing or leaking payloads", async () => {
  await exercise("wrong-scope", async ({ result, outputPath }) => {
    assert.equal(result.code, 1)
    assert.equal(result.stderr, "Path-1 fresh relay capture failed; no MP-10 acceptance verdict.\n")
    assert.equal(result.stdout, "")
    await assertNoEvidence(outputPath)
  })
})

await suite.test("real CLI sanitizes kernel errors and writes no evidence", async () => {
  await exercise("error", async ({ result, outputPath }) => {
    assert.equal(result.code, 1)
    assert.equal(result.stderr, "Path-1 fresh relay capture failed; no MP-10 acceptance verdict.\n")
    assert.equal(result.stdout, "")
    await assertNoEvidence(outputPath)
  })
})

await suite.test("real CLI timeout writes no evidence and does not retry through a cached request", { timeout: 30_000 }, async () => {
  await exercise("timeout", async ({ result, outputPath }) => {
    assert.equal(result.code, 1)
    assert.equal(result.stderr, "Path-1 fresh relay capture failed; no MP-10 acceptance verdict.\n")
    assert.equal(result.stdout, "")
    await assertNoEvidence(outputPath)
  })
})
  } finally {
    if (buildRoot) await rm(buildRoot, { recursive: true, force: true })
  }
})
