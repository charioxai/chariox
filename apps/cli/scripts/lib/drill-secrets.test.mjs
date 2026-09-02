import assert from "node:assert/strict"
import test from "node:test"

import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"

test("detects sensitive drill metadata keys", () => {
  assert.equal(isSensitiveDrillKey("relayToken"), true)
  assert.equal(isSensitiveDrillKey("api_key"), true)
  assert.equal(isSensitiveDrillKey("provider"), false)
})

test("detects token-shaped drill metadata values", () => {
  assert.equal(looksLikeDrillSecretValue("Bearer abcdefghijklmnopqrstuvwxyz"), true)
  assert.equal(looksLikeDrillSecretValue("chariox-scoped-v1.claims.signature"), true)
  assert.equal(looksLikeDrillSecretValue("sk-this-should-not-persist"), true)
  assert.equal(looksLikeDrillSecretValue("ghp_abcdefghijklmnopqrstuvwxyz"), true)
  assert.equal(looksLikeDrillSecretValue("/tmp/chariox-drill"), false)
})

test("redacts token-shaped text without dropping surrounding diagnostics", () => {
  assert.equal(
    redactDrillSecretText("request failed with Bearer abcdefghijklmnopqrstuvwxyz in header"),
    "request failed with <redacted> in header",
  )
  assert.equal(
    redactDrillSecretText("relay rejected chariox-scoped-v1.claims.signature during reconnect"),
    "relay rejected <redacted> during reconnect",
  )
  assert.equal(redactDrillSecretText("no secret here"), "no secret here")
})

test("redacts every repeated secret of the same kind", () => {
  const cases = [
    [
      "Bearer abcdefghijklmnopqrstuvwxyz then Bearer zyxwvutsrqponmlkjihgfedcba",
      ["abcdefghijklmnopqrstuvwxyz", "zyxwvutsrqponmlkjihgfedcba"],
    ],
    [
      "chariox-scoped-v1.claims.signature then chariox-scoped-v1.other.signature",
      ["chariox-scoped-v1.claims.signature", "chariox-scoped-v1.other.signature"],
    ],
    [
      "sk-aaaaaaaaaaaaaaaa then sk-bbbbbbbbbbbbbbbb",
      ["sk-aaaaaaaaaaaaaaaa", "sk-bbbbbbbbbbbbbbbb"],
    ],
    [
      "ghp_aaaaaaaaaaaaaaaa then gho_bbbbbbbbbbbbbbbb",
      ["ghp_aaaaaaaaaaaaaaaa", "gho_bbbbbbbbbbbbbbbb"],
    ],
  ]

  for (const [input, secretValues] of cases) {
    const redacted = redactDrillSecretText(input)
    assert.equal((redacted.match(/<redacted>/g) ?? []).length, 2)
    for (const secretValue of secretValues) assert.equal(redacted.includes(secretValue), false)
  }
})

test("sanitizes nested drill metadata", () => {
  assert.deepEqual(sanitizeDrillMetadata({
    drill: "remote",
    relayToken: "secret",
    provider: "Bearer abcdefghijklmnopqrstuvwxyz",
    nested: {
      apiKey: "sk-this-should-not-persist",
      safe: "value",
    },
    list: ["safe", "ghp_abcdefghijklmnopqrstuvwxyz"],
    unsupported: undefined,
  }), {
    drill: "remote",
    relayToken: "<redacted>",
    provider: "<redacted>",
    nested: {
      apiKey: "<redacted>",
      safe: "value",
    },
    list: ["safe", "<redacted>"],
    unsupported: null,
  })
})

test("preserves non-secret cleanup results whose names mention credentials", () => {
  assert.deepEqual(sanitizeDrillMetadata({
    provider_credentials_removed: true,
    provider_credentials_cleanup_error: "Bearer abcdefghijklmnopqrstuvwxyz",
  }), {
    provider_credentials_removed: true,
    provider_credentials_cleanup_error: "<redacted>",
  })
})
