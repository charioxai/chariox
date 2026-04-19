import assert from "node:assert/strict"
import test from "node:test"

import { createInitialShellContext, defaultKernelEndpoint, parseShellCliArgs, shellUsage } from "./shell.js"

test("parseShellCliArgs parses kernel and context options", () => {
  assert.deepEqual(parseShellCliArgs([
    "--kernel-url", "ws://127.0.0.1:9999/kernel",
    "--workspace", "/repo",
    "--worktree", "/repo/wt",
    "--provider", "codex",
    "--model", "gpt-5.2",
    "--effort", "low",
  ]), {
    kernelUrl: "ws://127.0.0.1:9999/kernel",
    workspace: "/repo",
    worktree: "/repo/wt",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })
})

test("parseShellCliArgs rejects conflicting endpoints", () => {
  assert.throws(() => parseShellCliArgs(["--kernel-url", "ws://x", "--socket", "/tmp/k.sock"]), /cannot be used together/)
})

test("createInitialShellContext defaults worktree to workspace", () => {
  const context = createInitialShellContext({ workspace: "/repo", provider: "codex" })
  assert.equal(context.workspace, "/repo")
  assert.equal(context.worktree, "/repo")
  assert.equal(context.provider, "codex")
})

test("defaultKernelEndpoint honors env overrides", () => {
  const previousUrl = process.env.ARROBA_KERNEL_URL
  process.env.ARROBA_KERNEL_URL = "ws://example/kernel"
  try {
    assert.equal(defaultKernelEndpoint(), "ws://example/kernel")
  } finally {
    if (previousUrl === undefined) {
      delete process.env.ARROBA_KERNEL_URL
    } else {
      process.env.ARROBA_KERNEL_URL = previousUrl
    }
  }
})

test("shellUsage documents prompt commands without slash prefix", () => {
  const usage = shellUsage()
  assert.match(usage, /arroba-shell/)
  assert.match(usage, /@ session list/)
})
