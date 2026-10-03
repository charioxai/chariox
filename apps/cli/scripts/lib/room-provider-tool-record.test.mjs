// MP-08/MP-10: official native tools can encode JSON twice inside MCP text.
import assert from "node:assert/strict"
import test from "node:test"
import { roomProviderToolOutput } from "./room-provider-tool-record.mjs"

test("MP-08/MP-10 decodes a native structured business result", () => {
  const vendors = { vendors: [{ name: "Acme", cost: 350, warranty_years: 3 }] }
  const envelope = { content: [{ type: "text", text: JSON.stringify(JSON.stringify(vendors)) }], structuredContent: JSON.stringify(vendors) }
  assert.deepEqual(roomProviderToolOutput(JSON.stringify(envelope)), vendors)
})

test("MP-08/MP-10 keeps ambiguous and non-JSON output unverifiable", () => {
  assert.equal(roomProviderToolOutput({ content: [{ type: "text", text: "one" }, { type: "text", text: "two" }] }), null)
  assert.equal(roomProviderToolOutput({ content: [{ type: "text", text: "not JSON" }] }), null)
})
