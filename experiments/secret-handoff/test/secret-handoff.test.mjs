import { spawn } from "node:child_process"
import { createHash, createHmac, timingSafeEqual } from "node:crypto"
import http from "node:http"
import test from "node:test"
import assert from "node:assert/strict"

import {
  SecretHandoffHarness,
  passwordPromptPath,
} from "../src/secret-harness.mjs"

function kernelEnv() {
  return {
    PATH: process.env.PATH ?? "",
    HOME: process.env.HOME ?? "",
    TMPDIR: process.env.TMPDIR ?? "/tmp",
    OPENAI_API_KEY: "sk-test-secret",
    GITHUB_TOKEN: "ghp-test-secret",
    DB_PASSWORD: "db-password-secret",
    INTERNAL_HMAC_KEY: "hmac-secret",
  }
}

function harness() {
  return new SecretHandoffHarness({
    kernelEnv: kernelEnv(),
    credentials: {
      openai: {
        source: "env",
        env: "OPENAI_API_KEY",
        allowed_hosts: [],
        allowed_uses: ["http"],
        injection: {
          kind: "header",
          name: "authorization",
          value: "Bearer ${secret}",
        },
      },
      internal_hmac: {
        source: "env",
        env: "INTERNAL_HMAC_KEY",
        allowed_hosts: [],
        allowed_uses: ["http"],
        injection: {
          kind: "hmac",
          timestamp_header: "x-demo-timestamp",
          signature_header: "x-demo-signature",
        },
      },
      ssh_password: {
        source: "vault",
        value: "terminal-password",
        allowed_uses: ["pty"],
      },
    },
  })
}

test("env-source credentials are scrubbed from fake agent env", async () => {
  const runtime = harness()
  const inspected = await runtime.launchFakeAgent({
    names: ["OPENAI_API_KEY", "GITHUB_TOKEN", "DB_PASSWORD", "INTERNAL_HMAC_KEY", "PATH", "HOME", "TMPDIR"],
  })

  assert.equal(inspected.env.OPENAI_API_KEY, null)
  assert.equal(inspected.env.GITHUB_TOKEN, null)
  assert.equal(inspected.env.DB_PASSWORD, null)
  assert.equal(inspected.env.INTERNAL_HMAC_KEY, null)
  assert.ok(inspected.env.PATH)
  assert.ok(inspected.env.HOME)
})

test("bearer credential is injected into request without returning secret", async () => {
  let receivedAuthorization = null
  const server = await startServer((req, res) => {
    receivedAuthorization = req.headers.authorization
    res.writeHead(200, { "content-type": "application/json" })
    res.end(JSON.stringify({ ok: true }))
  })

  try {
    const runtime = harness()
    runtime.credentials.openai.allowed_hosts = [`127.0.0.1:${server.port}`]
    const result = await runtime.httpRequestWithCredential({
      credential_id: "openai",
      url: `http://127.0.0.1:${server.port}/echo`,
    })

    assert.equal(receivedAuthorization, "Bearer sk-test-secret")
    assert.deepEqual(result, { status: 200, body: { ok: true } })
    assert.equal(JSON.stringify(result).includes("sk-test-secret"), false)
  } finally {
    await server.close()
  }
})

test("wrong host is rejected before bearer injection", async () => {
  const runtime = harness()
  runtime.credentials.openai.allowed_hosts = ["api.openai.com"]
  await assert.rejects(
    runtime.httpRequestWithCredential({
      credential_id: "openai",
      url: "http://127.0.0.1:12345/echo",
    }),
    /not allowed for host/,
  )
})

test("generic HMAC signing works without exposing signing key", async () => {
  const server = await startServer((req, res, body) => {
    const timestamp = req.headers["x-demo-timestamp"]
    const signature = req.headers["x-demo-signature"]
    const url = new URL(req.url, `http://${req.headers.host}`)
    const bodyHash = createHash("sha256").update(body).digest("hex")
    const canonical = [
      req.method,
      `${url.pathname}${url.search}`,
      bodyHash,
      timestamp,
    ].join("\n")
    const expected = createHmac("sha256", "hmac-secret").update(canonical).digest("hex")
    const ok = safeEqualHex(String(signature), expected)
    res.writeHead(ok ? 200 : 401, { "content-type": "application/json" })
    res.end(JSON.stringify({ ok }))
  })

  try {
    const runtime = harness()
    runtime.credentials.internal_hmac.allowed_hosts = [`127.0.0.1:${server.port}`]
    const result = await runtime.httpRequestWithCredential({
      credential_id: "internal_hmac",
      method: "POST",
      url: `http://127.0.0.1:${server.port}/signed?mode=test`,
      body: { hello: "world" },
    })

    assert.equal(result.status, 200)
    assert.deepEqual(result.body, { ok: true })
    assert.equal(JSON.stringify(result).includes("hmac-secret"), false)
  } finally {
    await server.close()
  }
})

test("terminal password prompt receives secret without returning it", async () => {
  const runtime = harness()
  const child = spawn(process.execPath, [passwordPromptPath, "terminal-password"], {
    stdio: ["pipe", "pipe", "pipe"],
  })
  let stdout = ""
  child.stdout.on("data", (chunk) => { stdout += chunk.toString() })

  const result = await runtime.sendSecretToTerminal({
    credential_id: "ssh_password",
    child,
    pattern: "Password:",
  })
  const exitCode = await waitForExit(child)

  assert.deepEqual(result, { submitted: true })
  assert.equal(JSON.stringify(result).includes("terminal-password"), false)
  assert.equal(exitCode, 0)
  assert.match(stdout, /AUTH_OK/)
})

function safeEqualHex(a, b) {
  const left = Buffer.from(a, "hex")
  const right = Buffer.from(b, "hex")
  return left.length === right.length && timingSafeEqual(left, right)
}

async function startServer(handler) {
  const server = http.createServer((req, res) => {
    let body = ""
    req.setEncoding("utf8")
    req.on("data", (chunk) => { body += chunk })
    req.on("end", () => handler(req, res, body))
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  return {
    port: server.address().port,
    close: () => new Promise((resolve) => server.close(resolve)),
  }
}

function waitForExit(child) {
  return new Promise((resolve) => {
    child.once("exit", (code) => resolve(code))
  })
}
