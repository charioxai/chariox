// MP-07 / MP-08 / MP-11: exercise startup admission and dispatch without replacing CLI modules.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { once } from "node:events"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createServer } from "node:http"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { after, before, test } from "node:test"

const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
let root, compiled

async function run(executable, args, home) {
  const child = spawn(executable, args, {
    cwd: app,
    env: {
      PATH: `${path.join(home, ".local/bin")}${path.delimiter}${process.env.PATH}`,
      HOME: home, CHARIOX_HOME: path.join(home, "state"), CHARIOX_KERNEL_PORT: "0",
      CHARIOX_LOG_DIR: path.join(home, "logs"), XDG_CONFIG_HOME: path.join(home, "config"),
      XDG_DATA_HOME: path.join(home, "data"), XDG_CACHE_HOME: path.join(home, "cache"),
      TERM: "dumb", NO_COLOR: "1",
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = "", stderr = "", timedOut = false
  child.stdout.on("data", chunk => { stdout += chunk })
  child.stderr.on("data", chunk => { stderr += chunk })
  const timer = setTimeout(() => {
    timedOut = true
    if (Number.isSafeInteger(child.pid) && child.pid > 1) child.kill("SIGKILL")
  }, 30_000)
  try {
    const [code, signal] = await once(child, "close")
    assert.equal(timedOut, false, "CLI entry timed out")
    assert.equal(signal, null, "CLI entry terminated by signal")
    return { code, stdout, stderr }
  } finally { clearTimeout(timer) }
}

before(async context => {
  root = await mkdtemp(path.join(os.tmpdir(), "chariox-login-setup-entry-"))
  compiled = path.join(root, "chariox")
  const target = `${process.platform === "darwin" ? "darwin" : "linux"}-${process.arch}`
  const result = await run("bun", [path.join(app, "scripts/compile.mjs"), "--target", target, "--outfile", compiled], root)
  assert.equal(result.code, 0, result.stderr)
  context.diagnostic(`MP-07 / MP-08 / MP-11 compiled CLI sha256:${createHash("sha256").update(await readFile(compiled)).digest("hex")}`)
})
after(async () => { if (root) await rm(root, { recursive: true, force: true }) })

for (const entry of ["source-index", "source-release", "compiled-release"]) {
  // The normal source CLI runs Babel-built JS with Bun (package.json start/dev).
  // Raw TSX needs that Solid transform; the release executable embeds the same JS.
  const invoke = (args, home) => entry === "compiled-release"
    ? run(compiled, args, home)
    : run("bun", [path.join(app, "dist", entry === "source-index" ? "index.js" : "release-main.js"), ...args], home)

  test(`MP-07 / MP-08 / MP-11 ${entry}: login enrolls the terminal, offers setup, and setup launches its installer`, async () => {
    const home = await mkdtemp(path.join(root, `${entry}-login-`))
    await mkdir(path.join(home, ".local/bin"), { recursive: true })
    // Only the installer/Cloud are fixtures. Startup, parser, commands, identity
    // creation, credential persistence and setup offer are the actual product.
    await writeFile(path.join(home, ".local/bin/chariox-setup"), '#!/bin/sh\nprintf "%s\\n" "$@" > "$HOME/setup-args"\n', { mode: 0o700 })
    await writeFile(path.join(home, ".local/bin", process.platform === "darwin" ? "open" : "xdg-open"), '#!/bin/sh\nexit 0\n', { mode: 0o700 })
    const requests = []
    let clientId, thumbprint
    const server = createServer(async (request, response) => {
      const chunks = []
      for await (const chunk of request) chunks.push(chunk)
      const body = JSON.parse(Buffer.concat(chunks).toString())
      requests.push(request.url)
      response.setHeader("content-type", "application/json")
      if (request.url === "/auth/device/start") {
        assert.equal(body.enrollmentKind, "CLIENT", "login must enroll a terminal")
        clientId = body.clientId; thumbprint = body.publicKeyThumbprint
        response.writeHead(201).end(JSON.stringify({ deviceCode: "synthetic-device", userCode: "PUBLIC-CODE", verificationUrl: "http://127.0.0.1/activate", expiresAt: new Date(Date.now() + 60_000).toISOString(), intervalSeconds: 1 }))
      } else if (request.url === "/auth/device/poll") {
        response.end(JSON.stringify({ status: "approved", profile: { enrollmentKind: "CLIENT", publicKeyThumbprint: thumbprint, clientId, accountId: "fixture-account", userId: "fixture-owner", email: "fixture@example.test", accountSlug: "fixture", realmId: "fixture-realm", relayUrl: "ws://127.0.0.1:1", issuerId: "fixture" }, refreshCredential: "synthetic-refresh", cloudSessionToken: "synthetic-access", cloudSessionExpiresAt: new Date(Date.now() + 3_600_000).toISOString() }))
      } else {
        response.writeHead(404).end('{}')
      }
    })
    server.listen(0, "127.0.0.1")
    await once(server, "listening")
    const apiUrl = `http://127.0.0.1:${server.address().port}`
    try {
      const login = await invoke(["login", "--api-url", apiUrl], home)
      assert.equal(login.code, 0, login.stderr)
      assert.deepEqual(requests, ["/auth/device/start", "/auth/device/poll"])
      assert.match(login.stdout, /Terminal signed in: fixture/)
      assert.match(login.stdout, /Set up a Chariox kernel on this machine\? Run chariox setup/)
      const setup = await invoke(["setup", "--api-url", apiUrl], home)
      assert.equal(setup.code, 0, setup.stderr)
      assert.deepEqual((await readFile(path.join(home, "setup-args"), "utf8")).trim().split("\n"), ["--api-url", apiUrl, "--user-id", "fixture-owner"])
      assert.deepEqual(requests, ["/auth/device/start", "/auth/device/poll"], "setup uses the saved terminal profile")
    } finally {
      await new Promise(resolve => server.close(resolve))
      await rm(home, { recursive: true, force: true })
    }
  })

  test(`MP-08 / MP-11 ${entry}: signed-out setup reaches its login guard`, async () => {
    const home = await mkdtemp(path.join(root, `${entry}-signed-out-`))
    const result = await invoke(["setup"], home)
    assert.equal(result.code, 1)
    assert.match(result.stderr, /Sign in first with \/cloud login/)
    assert.doesNotMatch(result.stderr, /unknown argument setup/)
  })

  for (const command of ["login", "setup"]) {
    test(`MP-08 / MP-11 ${entry}: ${command} keeps its own argument validation`, async () => {
      const home = await mkdtemp(path.join(root, `${entry}-invalid-`))
      const result = await invoke([command, "--unknown"], home)
      assert.equal(result.code, 1)
      assert.match(result.stderr, /usage: chariox cloud/)
      assert.doesNotMatch(result.stderr, new RegExp(`unknown argument ${command}`))
    })
  }
}
