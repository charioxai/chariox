import { spawn } from "node:child_process"
import { createHash, createHmac } from "node:crypto"
import { readFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const srcRoot = path.dirname(fileURLToPath(import.meta.url))
export const fakeAgentPath = path.join(srcRoot, "fake-agent.mjs")
export const passwordPromptPath = path.join(srcRoot, "password-prompt.mjs")

const DEFAULT_ENV_ALLOWLIST = new Set([
  "PATH",
  "HOME",
  "USER",
  "SHELL",
  "TMPDIR",
  "TEMP",
  "TMP",
  "LANG",
  "PWD",
])

const SECRET_NAME_PATTERN = /(^|_)(TOKEN|SECRET|PASSWORD|PASS|API_KEY|PRIVATE_KEY|ACCESS_KEY)$/i

export class SecretHandoffHarness {
  constructor({ credentials = {}, kernelEnv = process.env, vault = {} } = {}) {
    this.credentials = credentials
    this.kernelEnv = { ...kernelEnv }
    this.vault = { ...vault }
  }

  credentialEnvNames() {
    return Object.values(this.credentials)
      .filter((credential) => credential.source === "env" && credential.env)
      .map((credential) => credential.env)
  }

  scrubProviderEnv(baseEnv = this.kernelEnv) {
    const credentialEnvNames = new Set(this.credentialEnvNames())
    const scrubbed = {}
    for (const [name, value] of Object.entries(baseEnv)) {
      const allowed = DEFAULT_ENV_ALLOWLIST.has(name) || name.startsWith("LC_")
      if (!allowed) continue
      if (credentialEnvNames.has(name)) continue
      if (SECRET_NAME_PATTERN.test(name)) continue
      scrubbed[name] = value
    }
    if (!scrubbed.PATH && process.env.PATH) scrubbed.PATH = process.env.PATH
    if (!scrubbed.HOME && process.env.HOME) scrubbed.HOME = process.env.HOME
    if (!scrubbed.TMPDIR && process.env.TMPDIR) scrubbed.TMPDIR = process.env.TMPDIR
    return scrubbed
  }

  async resolveSecret(credentialId) {
    const credential = this.credentials[credentialId]
    if (!credential) throw new Error(`unknown credential: ${credentialId}`)
    if (credential.source === "env") {
      const value = this.kernelEnv[credential.env]
      if (!value) throw new Error(`credential ${credentialId} env ${credential.env} is not set`)
      return value
    }
    if (credential.source === "vault") {
      const value = this.vault[credentialId] ?? credential.value
      if (!value) throw new Error(`credential ${credentialId} is not present in vault`)
      return value
    }
    if (credential.source === "file") {
      return (await readFile(credential.path, "utf8")).trimEnd()
    }
    throw new Error(`unsupported credential source for ${credentialId}: ${credential.source}`)
  }

  validateUse(credentialId, { url, use }) {
    const credential = this.credentials[credentialId]
    if (!credential) throw new Error(`unknown credential: ${credentialId}`)
    if (credential.allowed_uses && !credential.allowed_uses.includes(use)) {
      throw new Error(`credential ${credentialId} is not allowed for ${use}`)
    }
    if (url && credential.allowed_hosts?.length) {
      const host = new URL(url).host
      if (!credential.allowed_hosts.includes(host)) {
        throw new Error(`credential ${credentialId} is not allowed for host ${host}`)
      }
    }
    return credential
  }

  launchFakeAgent({ names = [] } = {}) {
    return runJson(process.execPath, [
      fakeAgentPath,
      "inspect-env",
      `--names=${names.join(",")}`,
    ], {
      env: this.scrubProviderEnv(),
    })
  }

  async httpRequestWithCredential({ credential_id, method = "GET", url, headers = {}, body = null }) {
    const credential = this.validateUse(credential_id, { url, use: "http" })
    const secret = await this.resolveSecret(credential_id)
    const requestHeaders = { ...headers }
    const target = new URL(url)
    const requestBody = body == null
      ? undefined
      : typeof body === "string"
        ? body
        : JSON.stringify(body)

    if (credential.injection?.kind === "header") {
      requestHeaders[credential.injection.name] = renderTemplate(credential.injection.value, secret)
    } else if (credential.injection?.kind === "query") {
      target.searchParams.set(credential.injection.name, secret)
    } else if (credential.injection?.kind === "basic") {
      const username = credential.injection.username ?? ""
      requestHeaders.authorization = `Basic ${Buffer.from(`${username}:${secret}`).toString("base64")}`
    } else if (credential.injection?.kind === "hmac") {
      const timestamp = String(Math.floor(Date.now() / 1000))
      const bodyHash = createHash("sha256").update(requestBody ?? "").digest("hex")
      const canonical = [
        method.toUpperCase(),
        `${target.pathname}${target.search}`,
        bodyHash,
        timestamp,
      ].join("\n")
      const signature = createHmac("sha256", secret).update(canonical).digest("hex")
      requestHeaders[credential.injection.timestamp_header ?? "x-arroba-timestamp"] = timestamp
      requestHeaders[credential.injection.signature_header ?? "x-arroba-signature"] = signature
    } else {
      throw new Error(`unsupported injection kind for ${credential_id}`)
    }

    const response = await fetch(target, {
      method,
      headers: requestHeaders,
      body: requestBody,
    })
    const text = await response.text()
    return {
      status: response.status,
      body: parseMaybeJson(text),
    }
  }

  async sendSecretToTerminal({ credential_id, child, pattern = "Password:", append_newline = true }) {
    this.validateUse(credential_id, { use: "pty" })
    const secret = await this.resolveSecret(credential_id)
    await waitForOutput(child, pattern)
    child.stdin.write(`${secret}${append_newline ? "\n" : ""}`)
    return { submitted: true }
  }
}

function renderTemplate(template, secret) {
  return String(template ?? "${secret}").replaceAll("${secret}", secret)
}

function parseMaybeJson(text) {
  if (!text) return null
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}

function runJson(command, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code) => {
      if (code !== 0) {
        reject(new Error(`process exited ${code}: ${stderr}`))
        return
      }
      resolve(JSON.parse(stdout))
    })
  })
}

function waitForOutput(child, pattern, timeoutMs = 5000) {
  return new Promise((resolve, reject) => {
    let output = ""
    const timeout = setTimeout(() => {
      cleanup()
      reject(new Error(`timed out waiting for terminal pattern ${pattern}`))
    }, timeoutMs)
    const onData = (chunk) => {
      output += chunk.toString()
      if (!output.includes(pattern)) return
      cleanup()
      resolve(output)
    }
    const onExit = () => {
      cleanup()
      reject(new Error(`terminal process exited before pattern ${pattern}`))
    }
    const cleanup = () => {
      clearTimeout(timeout)
      child.stdout.off("data", onData)
      child.off("exit", onExit)
    }
    child.stdout.on("data", onData)
    child.once("exit", onExit)
  })
}
