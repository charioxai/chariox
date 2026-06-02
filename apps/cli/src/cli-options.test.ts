import test from "node:test"
import assert from "node:assert/strict"

import {
  applyProviderPreferenceDefaults,
  parseArgs,
  resolveConfiguredCloudRelayApiUrl,
} from "./cli-options.js"

test("parseArgs applies terminal pairing links", () => {
  const payload = Buffer.from(JSON.stringify({
    relay_url: "wss://relay.example.test",
    relay_token: "relay-token",
    target_daemon_id: "kernel-1",
    target_daemon_alias: "devbox",
    terminal_id: "terminal-1",
  })).toString("base64url")

  assert.deepEqual(parseArgs([`arroba-terminal-pair-v1.${payload}`]), {
    clientId: "terminal-1",
    provider: "opencode",
    model: "default",
    accountProfile: "default",
    effort: "",
    relayUrl: "wss://relay.example.test",
    relayToken: "relay-token",
    targetDaemonId: "kernel-1",
    targetDaemonAlias: "devbox",
  })
})

test("parseArgs rejects invalid option combinations", () => {
  assert.throws(
    () => parseArgs(["--relay-url", "wss://relay.example.test"]),
    /--relay-url requires --relay-token/,
  )
  assert.throws(
    () => parseArgs(["--create-session", "--session", "session-1"]),
    /--create-session cannot be used together with --session/,
  )
})

test("parseArgs help lists remote runtime once next to kernel health", () => {
  const previousWrite = process.stdout.write
  const previousExit = process.exit
  let output = ""
  ;(process.stdout.write as unknown as (chunk: string) => boolean) = (chunk: string) => {
    output += chunk
    return true
  }
  ;(process.exit as unknown as (code?: number) => never) = ((code?: number) => {
    throw Object.assign(new Error("process.exit"), { code })
  }) as (code?: number) => never

  try {
    assert.throws(() => parseArgs(["--help"]), /process\.exit/)
  } finally {
    process.stdout.write = previousWrite
    process.exit = previousExit
  }

  assert.match(output, /\/kernel health\s+show runtime health, remote readiness, and invariants/)
  assert.match(output, /\/kernel remote-runtime\s+show provider runs, remote agents, slices, home-proxy, and live sync readiness/)
  assert.match(output, /\/slice auth import\s+copy host provider auth into this slice account/)
  assert.match(output, /\/slice auth login\s+start login for a slice-specific provider account/)
  assert.match(output, /\/slice auth alias\s+set or clear a slice provider account alias/)
  assert.equal(output.match(/\/kernel health/g)?.length, 1)
  assert.equal(output.match(/\/kernel remote-runtime/g)?.length, 1)
})

test("resolveConfiguredCloudRelayApiUrl prefers env and trims trailing slashes", () => {
  const previous = process.env.ARROBA_CLOUD_API_URL
  process.env.ARROBA_CLOUD_API_URL = "https://cloud.example.test///"
  try {
    assert.equal(
      resolveConfiguredCloudRelayApiUrl({
        relay: {
          cloud: {
            apiUrl: "https://profile.example.test",
            email: "user@example.test",
            accountId: "account-1",
            userId: "user-1",
            accountSlug: "account",
            realmId: "realm-1",
            relayUrl: "wss://relay.example.test",
            issuerId: "issuer-1",
          },
        },
      }),
      "https://cloud.example.test",
    )
  } finally {
    if (previous === undefined) {
      delete process.env.ARROBA_CLOUD_API_URL
    } else {
      process.env.ARROBA_CLOUD_API_URL = previous
    }
  }
})

test("applyProviderPreferenceDefaults fills default model and blank effort from provider preferences", () => {
  const options = parseArgs(["--provider", "codex"])

  assert.equal(
    applyProviderPreferenceDefaults(options, {
      providers: {
        codex: {
          model: "gpt-5.1",
          effort: "high",
        },
      },
    }).model,
    "gpt-5.1",
  )
  assert.equal(options.effort, "high")
})

test("applyProviderPreferenceDefaults preserves explicit model and effort", () => {
  const options = parseArgs(["--provider", "codex", "--model", "gpt-5.2", "--effort", "medium"])

  applyProviderPreferenceDefaults(options, {
    providers: {
      codex: {
        model: "gpt-5.1",
        effort: "high",
      },
    },
  })

  assert.equal(options.model, "gpt-5.2")
  assert.equal(options.effort, "medium")
})
