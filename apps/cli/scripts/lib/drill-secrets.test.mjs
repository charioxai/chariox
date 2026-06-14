import assert from "node:assert/strict"
import test from "node:test"

import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"

test("detects sensitive drill metadata keys", () => {
  assert.equal(isSensitiveDrillKey("relayToken"), true)
  assert.equal(isSensitiveDrillKey("api_key"), true)
  assert.equal(isSensitiveDrillKey("provider"), false)
})

test("detects token-shaped drill metadata values", () => {
  assert.equal(looksLikeDrillSecretValue("Bearer abcdefghijklmnopqrstuvwxyz"), true)
  assert.equal(looksLikeDrillSecretValue("sk-this-should-not-persist"), true)
  assert.equal(looksLikeDrillSecretValue("ghp_abcdefghijklmnopqrstuvwxyz"), true)
  assert.equal(looksLikeDrillSecretValue("/tmp/arroba-drill"), false)
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
