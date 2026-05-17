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
