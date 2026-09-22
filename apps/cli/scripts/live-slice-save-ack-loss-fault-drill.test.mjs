import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { createRunner } from "./lib/local-rust-fault-drill-runtime.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-slice-save-ack-loss-fault-drill.mjs", import.meta.url))

test("slice save acknowledgement-loss dry-run records an exact serial kernel command externally", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-slice-save-ack-loss-"))
  const reportPath = path.join(root, "report.json")
  try {
    await execFile(process.execPath, [scriptPath, "--dry-run", "--report", reportPath])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.schema, "chariox.slice_save_ack_loss_fault_drill.v1")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.caseIds, [
      "fault.response-loss",
      "effect.backend-exactly-once",
      "replay.same-process",
      "replay.kernel-restart",
      "guard.command-conflict",
      "cleanup.resources",
    ])
    assert.deepEqual(report.command.args.slice(0, 4), ["test", "-p", "chariox-kernel", "--lib"])
    assert.equal(report.command.env.CARGO_BUILD_JOBS, "1")
    assert(path.isAbsolute(report.command.env.CARGO_TARGET_DIR))
    assert.deepEqual(report.execution, {
      status: "not-run",
      exitCode: null,
      signal: null,
      timedOut: false,
      stdoutBytes: 0,
      stderrBytes: 0,
      failureLine: null,
    })
    assert.deepEqual(report.output, { stdoutTail: "", stderrTail: "" })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("fault-drill failed child preserves status and an early diagnostic after long warnings", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-slice-save-ack-loss-failure-"))
  const reportPath = path.join(root, "report.json")
  const binDir = path.join(root, "bin")
  const cargoPath = path.join(binDir, "cargo")
  try {
    await mkdir(binDir)
    await writeFile(cargoPath, [
      "#!/bin/sh",
      "echo 'error: SENTINEL cargo fixture failure' >&2",
      "i=0",
      "while [ \"$i\" -lt 300 ]; do",
      "  echo 'warning: intentionally long compiler warning output for bounded evidence' >&2",
      "  i=$((i + 1))",
      "done",
      "sleep 0.2",
      "exit 7",
      "",
    ].join("\n"))
    await chmod(cargoPath, 0o700)

    await assert.rejects(execFile(process.execPath, [scriptPath, "--report", reportPath], {
      env: { ...process.env, PATH: `${binDir}${path.delimiter}${process.env.PATH ?? ""}` },
    }))
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    assert.equal(report.status, "failed")
    assert.equal(report.execution.status, "failed")
    assert.equal(report.execution.exitCode, 7)
    assert.equal(report.execution.signal, null)
    assert.equal(report.execution.timedOut, false)
    assert(report.execution.stderrBytes > 4_000)
    assert.equal(report.execution.failureLine, "error: SENTINEL cargo fixture failure")
    assert.match(report.failure, /cargo exited with code 7/)
    assert(report.output.stderrTail.length <= 4_000)
    assert.equal(report.resources.length, 3)
    for (const sample of report.resources) {
      assert.equal(typeof sample.disk, "string")
      assert.equal(sample.diskPath, path.resolve(scriptPath, "..", "..", "..", ".."))
      assert(Number.isSafeInteger(sample.diskAvailableBytes))
      assert(sample.diskAvailableBytes > 0)
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("timed-out child is failed closed even when it exits zero after SIGTERM", async () => {
  const children = new Set()
  const run = createRunner({ repoRoot: process.cwd(), children })
  await assert.rejects(
    run(
      process.execPath,
      [
        "-e",
        "process.on('SIGTERM', () => {}); setTimeout(() => process.exit(0), 1000)",
      ],
      { timeoutMs: 250 },
    ),
    (error) => {
      assert.match(error.message, /exited with timeout/)
      assert.equal(error.result.code, 0)
      assert.equal(error.result.timedOut, true)
      return true
    },
  )
  assert.equal(children.size, 0)
})
