import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import { resolveProviderThreadDrillPaths } from "./provider-thread-drill-paths.mjs"

test("provider thread drill separates durable evidence from disposable runtime state", () => {
  assert.deepEqual(resolveProviderThreadDrillPaths({
    homeDir: "/Users/tester",
    runId: "provider-run",
    env: {},
  }), {
    evidenceRoot: path.join(
      "/Users/tester",
      ".codex/evidence/browser-computer-use/provider-thread-transfer/provider-run",
    ),
    runtimeRoot: path.join(
      "/Users/tester",
      ".chariox/dev/browser-computer-use-provider-thread-transfer/provider-run",
    ),
  })
})

test("provider thread drill accepts explicit evidence and runtime roots", () => {
  assert.deepEqual(resolveProviderThreadDrillPaths({
    homeDir: "/Users/tester",
    runId: "provider-run",
    env: {
      CHARIOX_PROVIDER_THREAD_EVIDENCE_ROOT: "/tmp/evidence",
      CHARIOX_PROVIDER_THREAD_RUNTIME_ROOT: "/tmp/runtime",
    },
  }), {
    evidenceRoot: "/tmp/evidence",
    runtimeRoot: "/tmp/runtime",
  })
})

test("provider thread drill rejects unsafe root overrides", () => {
  assert.throws(
    () => resolveProviderThreadDrillPaths({
      homeDir: "/Users/tester",
      runId: "provider-run",
      env: { CHARIOX_PROVIDER_THREAD_RUNTIME_ROOT: "relative/runtime" },
    }),
    /absolute/,
  )
  assert.throws(
    () => resolveProviderThreadDrillPaths({
      homeDir: "/Users/tester",
      runId: "provider-run",
      env: {
        CHARIOX_PROVIDER_THREAD_EVIDENCE_ROOT: "/tmp/shared",
        CHARIOX_PROVIDER_THREAD_RUNTIME_ROOT: "/tmp/shared",
      },
    }),
    /must differ/,
  )
})
