import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { delimiter, join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

import {
  clippyComparisonBase,
  findWarningRegressions,
  warningFromCompilerEvent,
  warningIdentity,
} from "./clippy-changed-lines-gate.mjs"

test("final-head manual CI compares the complete branch, not just its last commit", () => {
  assert.equal(clippyComparisonBase(undefined, { GITHUB_EVENT_NAME: "workflow_dispatch" }), "origin/main")
  assert.equal(clippyComparisonBase(undefined, { GITHUB_BASE_REF: "release" }), "origin/release")
  assert.equal(clippyComparisonBase("reviewed-sha", { GITHUB_EVENT_NAME: "workflow_dispatch" }), "reviewed-sha")
  assert.equal(clippyComparisonBase(undefined, {}), "origin/main")
  assert.equal(clippyComparisonBase(undefined, { GITHUB_EVENT_NAME: "push" }), "HEAD^")
})

test("Clippy and lint reject stale origin/main and pass after fetching it", { skip: process.platform === "win32" }, (t) => {
  const root = mkdtempSync(join(tmpdir(), "main-baseline-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const origin = join(root, "origin")
  const checkout = join(root, "checkout")
  const bin = join(root, "bin")
  const invocations = join(root, "cargo-invocations")
  const git = (cwd, ...args) => execFileSync("git", [
    "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", ...args,
  ], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim()

  git(root, "init", "-q", "-b", "main", origin)
  git(origin, "commit", "-q", "--allow-empty", "-m", "Initial main")
  git(root, "clone", "-q", origin, checkout)
  git(checkout, "switch", "-q", "-c", "topic")
  git(checkout, "commit", "-q", "--allow-empty", "-m", "Topic change")
  git(origin, "commit", "-q", "--allow-empty", "-m", "Advance main")

  mkdirSync(bin)
  writeFileSync(join(bin, "rustup"), '#!/bin/sh\nprintf "fixture-toolchain\\n"\n', { mode: 0o755 })
  writeFileSync(join(bin, "cargo"), '#!/bin/sh\ngit rev-parse HEAD >> "$CARGO_INVOCATIONS"\n', { mode: 0o755 })
  const env = {
    ...process.env,
    PATH: [bin, process.env.PATH].join(delimiter),
    CARGO_INVOCATIONS: invocations,
    CI: "",
    CHARIOX_BOUNDARY_BASE: "",
    GITHUB_EVENT_NAME: "",
    GITHUB_BASE_REF: "",
  }
  const run = (script, ...args) => spawnSync(process.execPath, [
    fileURLToPath(new URL(script, import.meta.url)), ...args,
  ], { cwd: checkout, env, encoding: "utf8", timeout: 15_000 })
  const commands = [
    ["./clippy-changed-lines-gate.mjs", "origin/main"],
    ["./checks/run.mjs", "--root", checkout, "--only", "commit-message-rules"],
  ]

  for (const [script, ...args] of commands) {
    const stale = run(script, ...args)
    assert.equal(stale.status, 1, stale.stderr || stale.stdout)
    assert.match(stale.stderr, /origin\/main.*stale/i)
    assert.match(stale.stderr, /git fetch origin main/)
  }
  assert.equal(existsSync(invocations), false, "Clippy should stop before invoking Cargo")

  git(checkout, "fetch", "-q", "origin", "main")
  const staleLocalBranch = run("./clippy-changed-lines-gate.mjs", "main")
  assert.equal(staleLocalBranch.status, 1, staleLocalBranch.stderr || staleLocalBranch.stdout)
  assert.match(staleLocalBranch.stderr, /main is stale.*git fetch origin main/)
  assert.equal(existsSync(invocations), false)

  for (const [script, ...args] of commands) {
    const current = run(script, ...args)
    assert.equal(current.status, 0, current.stderr || current.stdout)
  }
  assert.deepEqual(readFileSync(invocations, "utf8").trim().split("\n"), [
    git(checkout, "rev-parse", "HEAD"),
    git(origin, "rev-parse", "main"),
  ])
})

function warning(file, line, code, message) {
  return { file, line, column: 1, code, message }
}

test("normalizes a compiler warning without tying identity to its line", () => {
  const diagnostic = warningFromCompilerEvent({
    reason: "compiler-message",
    message: {
      level: "warning",
      code: { code: "dead_code" },
      message: "function `unused` is never used",
      spans: [{ file_name: "src/lib.rs", line_start: 12, column_start: 4, is_primary: true }],
    },
  })

  assert.deepEqual(diagnostic, {
    ...warning("src/lib.rs", 12, "dead_code", "function `unused` is never used"),
    column: 4,
  })
  assert.equal(
    warningIdentity(diagnostic),
    warningIdentity(warning("src/lib.rs", 99, "dead_code", "function `unused` is never used")),
  )
})

test("detects a deletion-induced warning whose span is on an unchanged line", () => {
  const regression = warning("src/lib.rs", 4, "dead_code", "function `now_unused` is never used")
  assert.deepEqual(findWarningRegressions([], [regression]), [regression])
})

test("compares duplicate diagnostics as a multiset", () => {
  const existing = warning("src/lib.rs", 4, "dead_code", "function `unused` is never used")
  const duplicate = warning("src/lib.rs", 40, "dead_code", "function `unused` is never used")

  assert.deepEqual(findWarningRegressions([existing], [existing, duplicate]), [duplicate])
  assert.deepEqual(findWarningRegressions([existing], [duplicate]), [])
})
