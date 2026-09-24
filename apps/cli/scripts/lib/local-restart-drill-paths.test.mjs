import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import { resolveLocalRestartDrillPaths } from "./local-restart-drill-paths.mjs"

test("local restart drill separates evidence, runtime state, and shared builds", () => {
  assert.deepEqual(resolveLocalRestartDrillPaths({
    homeDir: "/Users/tester",
    runId: "restart-run",
    env: {},
  }), {
    evidenceRoot: path.join(
      "/Users/tester",
      ".codex/evidence/browser-computer-use/local-restart-persistence/restart-run",
    ),
    runtimeRoot: path.join(
      "/Users/tester",
      ".chariox/dev/browser-computer-use-local-restart-persistence/restart-run",
    ),
    cargoTargetDir: path.join(
      "/Users/tester",
      ".chariox/dev/browser-computer-use/cargo-target",
    ),
  })
})

test("local restart drill accepts explicit absolute roots", () => {
  assert.deepEqual(resolveLocalRestartDrillPaths({
    homeDir: "/Users/tester",
    runId: "restart-run",
    env: {
      CHARIOX_LOCAL_RESTART_EVIDENCE_ROOT: "/tmp/restart-evidence",
      CHARIOX_LOCAL_RESTART_RUNTIME_ROOT: "/tmp/restart-runtime",
      CHARIOX_LOCAL_RESTART_CARGO_TARGET_DIR: "/tmp/restart-target",
    },
  }), {
    evidenceRoot: "/tmp/restart-evidence",
    runtimeRoot: "/tmp/restart-runtime",
    cargoTargetDir: "/tmp/restart-target",
  })
})

test("local restart drill rejects broad and overlapping roots", () => {
  assert.throws(
    () => resolveLocalRestartDrillPaths({
      homeDir: "/Users/tester",
      runId: "restart-run",
      env: { CHARIOX_LOCAL_RESTART_RUNTIME_ROOT: "/Users/tester" },
    }),
    /too broad/,
  )
  assert.throws(
    () => resolveLocalRestartDrillPaths({
      homeDir: "/Users/tester",
      runId: "restart-run",
      env: {
        CHARIOX_LOCAL_RESTART_EVIDENCE_ROOT: "/tmp/shared",
        CHARIOX_LOCAL_RESTART_RUNTIME_ROOT: "/tmp/shared/runtime",
      },
    }),
    /must differ and must not overlap/,
  )
  assert.throws(
    () => resolveLocalRestartDrillPaths({
      homeDir: "/Users/tester",
      runId: "restart-run",
      env: {
        CHARIOX_LOCAL_RESTART_RUNTIME_ROOT: "/tmp/shared/runtime",
        CHARIOX_LOCAL_RESTART_CARGO_TARGET_DIR: "/tmp/shared/runtime/cargo-target",
      },
    }),
    /must differ and must not overlap/,
  )
})
