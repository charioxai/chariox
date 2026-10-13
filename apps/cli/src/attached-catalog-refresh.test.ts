import assert from "node:assert/strict"
import test from "node:test"
import { refreshAttachedCatalogs } from "./attached-catalog-refresh.js"

// MP-08 / MP-10: stale catalog responses must not replace a newer attachment's choices.
test("catalog responses and failures are ignored after the attachment changes", async () => {
  let current = true
  let release!: (value: string) => void
  let fail!: (error: Error) => void
  const provider = new Promise<string>((resolve) => { release = resolve })
  const commands = new Promise<string>((_resolve, reject) => { fail = reject })
  const applied: string[] = []
  const warnings: string[] = []
  const loading = refreshAttachedCatalogs({ sessionId: "same-session", attachmentId: "old-attachment" }, {
    isCurrent: () => current,
    getProviderCatalog: () => provider,
    setProviderCatalog: (value) => applied.push(value),
    getCommandCatalog: () => commands,
    setCommandCatalog: (value) => applied.push(value),
    formatError: String,
    logWarning: (message) => warnings.push(message),
  })
  current = false
  release("stale catalog")
  fail(new Error("stale failure"))
  await loading
  assert.deepEqual(applied, [])
  assert.deepEqual(warnings, [])
})

test("catalogs update independently on the current attachment", async () => {
  let release!: (value: string) => void
  const provider = new Promise<string>((resolve) => { release = resolve })
  const applied: string[] = []
  const loading = refreshAttachedCatalogs({ sessionId: "session", attachmentId: "attachment" }, {
    isCurrent: () => true,
    getProviderCatalog: () => provider,
    setProviderCatalog: (value) => applied.push(value),
    getCommandCatalog: async () => "commands",
    setCommandCatalog: (value) => applied.push(value),
    formatError: String,
  })
  await Promise.resolve()
  assert.deepEqual(applied, ["commands"])
  release("provider")
  await loading
  assert.deepEqual(applied, ["commands", "provider"])
})
