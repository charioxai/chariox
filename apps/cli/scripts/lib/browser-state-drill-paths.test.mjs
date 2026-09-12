import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import { resolveBrowserStateDrillPaths } from "./browser-state-drill-paths.mjs"

test("browser state drill keeps evidence and disposable runtime state outside repositories", () => {
  const paths = resolveBrowserStateDrillPaths({
    homeDir: "/Users/tester",
    runId: "persistence-run",
    stamp: "20260902T020000Z",
    env: {},
  })

  assert.deepEqual(paths, {
    artifactDir: path.join(
      "/Users/tester",
      ".codex/evidence/browser-computer-use/persistence/20260902T020000Z",
    ),
    tempRoot: path.join(
      "/Users/tester",
      ".chariox/dev/browser-computer-use-persistence/persistence-run",
    ),
  })
})

test("browser state drill accepts explicit evidence and runtime roots", () => {
  const paths = resolveBrowserStateDrillPaths({
    homeDir: "/Users/tester",
    runId: "persistence-run",
    stamp: "20260902T020000Z",
    env: {
      M20_ARTIFACT_DIR: "/tmp/evidence",
      M20_RUNTIME_ROOT: "/tmp/runtime",
    },
  })

  assert.deepEqual(paths, {
    artifactDir: "/tmp/evidence",
    tempRoot: "/tmp/runtime",
  })
})
