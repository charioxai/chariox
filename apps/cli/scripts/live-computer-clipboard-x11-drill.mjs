#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { randomUUID, createHash } from "node:crypto"
import { access, chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  assertRetainedClipboardEvidenceIsRedacted,
  clipboardCaseSummary,
  clipboardInterruptionWindowMs,
  redactClipboardValue,
  utf8TextFromChunks,
} from "./lib/computer-clipboard-x11-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const drillScript = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const screenTool = path.join(
  repoRoot,
  "apps",
  "kernel",
  "slice-linux-docker",
  "docker",
  "slice-screen.sh",
)
const image = process.env.CHARIOX_SLICE_IMAGE ?? "chariox-slice-linux:0.1.0"
const runId = `${new Date().toISOString().replaceAll(":", "-")}-${process.pid}`
const containerName = `chariox-computer-clipboard-x11-${process.pid}`
const evidenceRoot = path.join(
  os.homedir(),
  ".codex",
  "evidence",
  "browser-computer-use",
  "computer-clipboard-x11",
  runId,
)
const containerRoot = "/tmp/chariox-clipboard-x11"
const containerProfile = `${containerRoot}/profile`
const containerTemp = `${containerRoot}/tmp`
const resourceLimits = {
  memoryBytes: 805_306_368,
  memorySwapBytes: 805_306_368,
  nanoCpus: 1_000_000_000,
  pidsLimit: 256,
}
const interruptionWindowMs = clipboardInterruptionWindowMs(
  process.env.CHARIOX_COMPUTER_CLIPBOARD_INTERRUPT_WINDOW_MS,
)

const children = new Set()
const commandOutput = []
let containerCreated = false
let unexpectedClipboardOutput = false
let interruptedSignal = null

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, () => {
    interruptedSignal ??= signal
    for (const child of children) child.kill("SIGTERM")
  })
}

function throwIfInterrupted() {
  if (interruptedSignal) {
    throw new Error(`drill interrupted by ${interruptedSignal}`)
  }
}

async function waitForInterruptionWindow(milliseconds) {
  const deadline = Date.now() + milliseconds
  while (Date.now() < deadline) {
    throwIfInterrupted()
    await new Promise((resolve) => setTimeout(resolve, Math.min(100, deadline - Date.now())))
  }
  throwIfInterrupted()
}

function redactAll(value, clipboardValues) {
  return clipboardValues.reduce(
    (redacted, clipboardValue) => redactClipboardValue(redacted, clipboardValue),
    value,
  )
}

function run(
  command,
  args,
  {
    input = null,
    timeoutMs = 120_000,
    clipboardValues = [],
    allowClipboardOutput = false,
  } = {},
) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
    })
    children.add(child)
    const stdoutChunks = []
    const stderrChunks = []
    const timeout = setTimeout(() => {
      child.kill("SIGTERM")
      setTimeout(() => child.kill("SIGKILL"), 2_000).unref()
    }, timeoutMs)
    child.stdout.on("data", (chunk) => {
      stdoutChunks.push(chunk)
    })
    child.stderr.on("data", (chunk) => {
      stderrChunks.push(chunk)
    })
    child.once("error", (error) => {
      clearTimeout(timeout)
      children.delete(child)
      reject(error)
    })
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      children.delete(child)
      const stdout = utf8TextFromChunks(stdoutChunks)
      const stderr = utf8TextFromChunks(stderrChunks)
      const output = `${stdout}${stderr}`
      if (
        !allowClipboardOutput &&
        clipboardValues.some(
          (clipboardValue) => clipboardValue.length > 0 && output.includes(clipboardValue),
        )
      ) {
        unexpectedClipboardOutput = true
      }
      commandOutput.push(redactAll(output, clipboardValues))
      resolve({ command, args, code, signal, stdout, stderr })
    })
    child.stdin.end(input ?? undefined)
  })
}

async function requireRun(command, args, options = {}) {
  throwIfInterrupted()
  const result = await run(command, args, options)
  throwIfInterrupted()
  if (result.code !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed with exit ${result.code}${
        result.signal ? ` (${result.signal})` : ""
      }\n${redactAll(`${result.stdout}${result.stderr}`, options.clipboardValues ?? [])}`,
    )
  }
  return result
}

async function docker(args, options = {}) {
  return await requireRun("docker", args, options)
}

async function waitFor(check, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    throwIfInterrupted()
    try {
      const value = await check()
      if (value) return value
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`${label} timed out${lastError ? `: ${lastError.message}` : ""}`)
}

async function resourceSnapshot(label) {
  const disk = await run("df", ["-k", repoRoot], { timeoutMs: 10_000 })
  const memoryPressure = await run("memory_pressure", ["-Q"], { timeoutMs: 10_000 }).catch(
    () => null,
  )
  const stats = containerCreated
    ? await run(
        "docker",
        ["stats", "--no-stream", "--format", "{{json .}}", containerName],
        { timeoutMs: 20_000 },
      ).catch(() => null)
    : null
  return {
    label,
    at: new Date().toISOString(),
    freeMemoryBytes: os.freemem(),
    loadAverage: os.loadavg(),
    disk: disk.stdout.trim().split("\n").at(-1),
    memoryPressure: memoryPressure?.code === 0 ? memoryPressure.stdout.trim() : null,
    dockerStats: stats?.code === 0 ? stats.stdout.trim() : null,
  }
}

async function cleanup(tempRoot) {
  for (const child of children) child.kill("SIGTERM")
  const containerRemoval = await run("docker", ["rm", "-f", containerName], {
    timeoutMs: 30_000,
  }).catch((error) => ({ code: null, error }))
  containerCreated = false
  let tempRootError = null
  try {
    await rm(tempRoot, { recursive: true, force: true })
  } catch (error) {
    tempRootError = error instanceof Error ? error.message : String(error)
  }
  return {
    containerRemoveExitCode: containerRemoval.code,
    containerRemoveError:
      "error" in containerRemoval && containerRemoval.error instanceof Error
        ? containerRemoval.error.message
        : null,
    tempRootError,
  }
}

async function pathMissing(target) {
  try {
    await access(target)
    return false
  } catch (error) {
    if (error?.code === "ENOENT") return true
    throw error
  }
}

function helperArgs(command, extraEnvironment = []) {
  return [
    "exec",
    "-i",
    "-u",
    "slice",
    "-e",
    "DISPLAY=:99",
    "-e",
    `TMPDIR=${containerTemp}`,
    "-e",
    `CHARIOX_SLICE_CHROME_PROFILE=${containerProfile}`,
    ...extraEnvironment,
    containerName,
    `${containerRoot}/slice-screen.sh`,
    command,
  ]
}

async function assertNoClipboardTempFiles() {
  const result = await docker([
    "exec",
    "-u",
    "slice",
    containerName,
    "/bin/bash",
    "-lc",
    `find ${containerTemp} -mindepth 1 -print`,
  ])
  assert.equal(result.stdout, "", "clipboard helper left content in its dedicated temp directory")
}

const exactLogNeedleScript = [
  "const fs=require('node:fs');",
  "const path=require('node:path');",
  "const root=process.argv[1];",
  "const chunks=[];",
  "process.stdin.on('data',chunk=>chunks.push(chunk));",
  "process.stdin.on('end',()=>{",
  "const needle=Buffer.concat(chunks);",
  "for(const name of fs.readdirSync(root)){",
  "if(!name.endsWith('.log'))continue;",
  "const target=path.join(root,name);",
  "if(fs.statSync(target).isFile()&&fs.readFileSync(target).includes(needle))process.stdout.write(target+'\\n');",
  "}",
  "});",
].join("")

async function collectDiagnostics(clipboardValues) {
  if (!containerCreated) return "container was not created"
  const result = await run(
    "docker",
    [
      "exec",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      `printf '%s\\n' '[processes]'; ps -eo pid,rss,stat,args; printf '%s\\n' '[files]'; find ${containerRoot} -maxdepth 2 -type f -print; for log in ${containerRoot}/*.log; do printf '[log %s]\\n' "$log"; tail -n 80 "$log"; done`,
    ],
    { timeoutMs: 20_000, clipboardValues },
  )
  return redactAll(`${result.stdout}${result.stderr}`, clipboardValues)
}

async function main() {
  const canary = randomUUID().replaceAll("-", "")
  const unicode = `clipboard-${canary}-Grüße 世界\nsecond line\n`
  const whitespace = `  clipboard-${canary}\t\n\n`
  const boundary = `clipboard-${canary}-`.padEnd(256 * 1024, "x")
  const clipboardCases = [
    { name: "empty", value: "", reads: 1 },
    { name: "unicode-newlines", value: unicode, reads: 2 },
    { name: "leading-trailing-whitespace", value: whitespace, reads: 1 },
    { name: "maximum-utf8-bytes", value: boundary, reads: 1 },
  ]
  const failingValue = `clipboard-failure-${canary}`
  const clipboardValues = [
    ...clipboardCases.map(({ value }) => value),
    failingValue,
    canary,
  ].filter((value) => value.length > 0)
  const reportPath = path.join(evidenceRoot, "report.json")
  const resources = []
  const sourceCommit = await requireRun("git", ["rev-parse", "HEAD"], { timeoutMs: 10_000 })
  const report = {
    schema: "chariox.computer_clipboard_x11_drill.v1",
    startedAt: new Date().toISOString(),
    command: "pnpm --dir apps/cli computer-clipboard:x11-drill",
    sourceCommits: { oss: sourceCommit.stdout.trim(), cloud: null },
    drillScriptSha256: createHash("sha256").update(await readFile(drillScript)).digest("hex"),
    screenToolSha256: createHash("sha256").update(await readFile(screenTool)).digest("hex"),
    image,
    topology: {
      kind: "local-docker-x11",
      host: os.hostname(),
      hostPlatform: process.platform,
      hostRelease: os.release(),
      hostArchitecture: os.arch(),
      containerNetwork: "none",
    },
    components: {
      node: process.version,
      provider: "not exercised",
      kernel: "not exercised; covered by clipboard protocol and encrypted worker tests",
      relay: "not exercised",
      cloud: "not exercised",
      selkies: "not exercised",
    },
    resourceLimits,
    cases: clipboardCases.map(({ name, value }) => clipboardCaseSummary(name, value)),
    checks: [],
    resources,
  }

  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-computer-clipboard-x11-"))
  const failureXclip = path.join(tempRoot, "xclip")
  let failure = null
  try {
    await mkdir(evidenceRoot, { recursive: true })
    await writeFile(failureXclip, "#!/bin/sh\ncat >/dev/null\nexit 17\n", "utf8")
    await chmod(failureXclip, 0o700)
    resources.push(await resourceSnapshot("before"))

    const dockerVersion = await docker(
      ["version", "--format", "client={{.Client.Version}} server={{.Server.Version}}"],
      { timeoutMs: 20_000 },
    )
    report.components.docker = dockerVersion.stdout.trim()
    const imageInspect = await docker(
      ["image", "inspect", "--format", "{{.Id}}", image],
      { timeoutMs: 20_000 },
    )
    report.imageId = imageInspect.stdout.trim()

    await docker([
      "create",
      "--name",
      containerName,
      "--memory",
      "768m",
      "--memory-swap",
      "768m",
      "--cpus",
      "1",
      "--pids-limit",
      "256",
      "--network",
      "none",
      "--entrypoint",
      "/bin/sleep",
      image,
      "infinity",
    ])
    containerCreated = true
    await docker(["start", containerName])
    if (interruptionWindowMs > 0) {
      await waitForInterruptionWindow(interruptionWindowMs)
    }

    const inspect = await docker([
      "container",
      "inspect",
      "--format",
      "{{json .HostConfig}}",
      containerName,
    ])
    const hostConfig = JSON.parse(inspect.stdout)
    assert.equal(hostConfig.Memory, resourceLimits.memoryBytes)
    assert.equal(hostConfig.MemorySwap, resourceLimits.memorySwapBytes)
    assert.equal(hostConfig.NanoCpus, resourceLimits.nanoCpus)
    assert.equal(hostConfig.PidsLimit, resourceLimits.pidsLimit)
    assert.equal(hostConfig.NetworkMode, "none")
    report.observedResourceLimits = {
      memoryBytes: hostConfig.Memory,
      memorySwapBytes: hostConfig.MemorySwap,
      nanoCpus: hostConfig.NanoCpus,
      pidsLimit: hostConfig.PidsLimit,
      networkMode: hostConfig.NetworkMode,
    }

    const containerVersions = await docker([
      "exec",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      ". /etc/os-release; printf 'os=%s-%s\\n' \"$ID\" \"$VERSION_ID\"; chromium --version; xclip -version 2>&1 | head -n 1",
    ])
    const [containerOs, browser, xclip] = containerVersions.stdout.trim().split("\n")
    report.components.containerOs = containerOs
    report.components.browser = browser
    report.components.xclip = xclip

    await docker([
      "exec",
      "-u",
      "root",
      containerName,
      "mkdir",
      "-p",
      containerRoot,
      containerTemp,
      `${containerRoot}/fail-bin`,
    ])
    await docker(["cp", screenTool, `${containerName}:${containerRoot}/slice-screen.sh`])
    await docker(["cp", failureXclip, `${containerName}:${containerRoot}/fail-bin/xclip`])
    await docker([
      "exec",
      "-u",
      "root",
      containerName,
      "chown",
      "-R",
      "slice:slice",
      containerRoot,
    ])

    await docker([
      "exec",
      "-d",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      `Xvfb :99 -screen 0 1280x800x24 -ac +extension RANDR +extension XTEST >${containerRoot}/xvfb.log 2>&1`,
    ])
    await waitFor(async () => {
      const result = await run("docker", [
        "exec",
        "-u",
        "slice",
        "-e",
        "DISPLAY=:99",
        containerName,
        "xdpyinfo",
      ])
      return result.code === 0
    }, "Xvfb readiness")
    await docker([
      "exec",
      "-d",
      "-u",
      "slice",
      "-e",
      "DISPLAY=:99",
      containerName,
      "/bin/bash",
      "-lc",
      `openbox >${containerRoot}/openbox.log 2>&1`,
    ])
    await docker([
      "exec",
      "-d",
      "-u",
      "slice",
      "-e",
      "DISPLAY=:99",
      containerName,
      "/bin/bash",
      "-lc",
      `exec chromium --user-data-dir=${containerProfile} --no-sandbox --password-store=basic --no-first-run --no-default-browser-check --disable-sync --disable-dev-shm-usage --disable-gpu --disable-background-networking --window-size=1280,800 about:blank >${containerRoot}/chromium.log 2>&1`,
    ])
    await waitFor(async () => {
      const result = await run("docker", [
        "exec",
        "-u",
        "slice",
        containerName,
        "/bin/bash",
        "-lc",
        `pgrep -af 'chromium.*${containerProfile}' | grep -v 'pgrep -af'`,
      ])
      return result.code === 0 && result.stdout.trim().length > 0
    }, "Chromium readiness", 45_000)

    for (const clipboardCase of clipboardCases) {
      const caseNeedles = clipboardCase.value.length > 0 ? [clipboardCase.value, canary] : []
      const write = await docker(helperArgs("computer-clipboard-write-stdin"), {
        input: clipboardCase.value,
        clipboardValues: caseNeedles,
        timeoutMs: 20_000,
      })
      assert.equal(write.stdout, "", `${clipboardCase.name} write emitted stdout`)
      assert.equal(write.stderr, "", `${clipboardCase.name} write emitted stderr`)
      await assertNoClipboardTempFiles()
      for (let read = 0; read < clipboardCase.reads; read += 1) {
        const observed = await docker(helperArgs("computer-clipboard-read"), {
          clipboardValues: caseNeedles,
          allowClipboardOutput: true,
          timeoutMs: 20_000,
        })
        assert.equal(observed.stdout, clipboardCase.value, `${clipboardCase.name} read was not exact`)
        assert.equal(observed.stderr, "", `${clipboardCase.name} read emitted stderr`)
      }
    }

    const failedWrite = await run(
      "docker",
      helperArgs("computer-clipboard-write-stdin", [
        "-e",
        `PATH=${containerRoot}/fail-bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
      ]),
      { input: failingValue, clipboardValues: [failingValue, canary], timeoutMs: 20_000 },
    )
    assert.equal(failedWrite.code, 17, "forced xclip failure did not propagate")
    assert.equal(failedWrite.stdout, "", "failed clipboard write emitted stdout")
    assert.equal(failedWrite.stderr, "", "failed clipboard write emitted stderr")
    await assertNoClipboardTempFiles()

    for (const clipboardValue of clipboardValues) {
      const logs = await docker(
        [
          "exec",
          "-i",
          "-u",
          "slice",
          containerName,
          "node",
          "-e",
          exactLogNeedleScript,
          containerRoot,
        ],
        { input: clipboardValue, clipboardValues: [clipboardValue] },
      )
      assert.equal(logs.stdout, "", "slice logs retained clipboard text")
    }
    assert.equal(unexpectedClipboardOutput, false, "a non-read command emitted clipboard text")
    assertRetainedClipboardEvidenceIsRedacted({ report, commandOutput }, unicode)
    assertRetainedClipboardEvidenceIsRedacted({ report, commandOutput }, whitespace)
    assertRetainedClipboardEvidenceIsRedacted({ report, commandOutput }, boundary)
    assertRetainedClipboardEvidenceIsRedacted({ report, commandOutput }, failingValue)

    report.checks.push(
      { name: "existing-slice-image", status: "passed" },
      { name: "resource-bounded-container", status: "passed" },
      { name: "real-x11-clipboard", status: "passed" },
      { name: "exact-empty-unicode-whitespace-and-boundary-bytes", status: "passed" },
      { name: "repeat-read-preserves-clipboard-owner", status: "passed" },
      { name: "failed-write-removes-plaintext-temp", status: "passed" },
      { name: "helper-output-and-log-leak-scan", status: "passed" },
    )
    resources.push(await resourceSnapshot("during"))
    report.status = "passed"
  } catch (error) {
    report.status = "failed"
    report.error = redactAll(error instanceof Error ? error.message : String(error), clipboardValues)
    report.diagnostics = await collectDiagnostics(clipboardValues).catch((diagnosticError) =>
      redactAll(
        diagnosticError instanceof Error ? diagnosticError.message : String(diagnosticError),
        clipboardValues,
      ),
    )
    failure = error
  }

  const cleanupAttempt = await cleanup(tempRoot)
  resources.push(await resourceSnapshot("after"))
  const leftovers = await run("docker", [
    "ps",
    "-a",
    "--filter",
    `name=^/${containerName}$`,
    "--format",
    "{{.Names}}",
  ]).catch(() => null)
  const containerRemoved = leftovers?.code === 0 && leftovers.stdout.trim() === ""
  const tempRootRemoved = await pathMissing(tempRoot)
  report.cleanup = { ...cleanupAttempt, containerRemoved, tempRootRemoved }
  if (!containerRemoved || !tempRootRemoved) {
    const cleanupError = new Error("drill cleanup left a container or temporary directory")
    report.status = "failed"
    report.error ??= cleanupError.message
    failure ??= cleanupError
  }
  report.completedAt = new Date().toISOString()
  let serializedReport = `${JSON.stringify(report, null, 2)}\n`
  serializedReport = redactAll(serializedReport, clipboardValues)
  for (const clipboardValue of clipboardValues) {
    assertRetainedClipboardEvidenceIsRedacted(serializedReport, clipboardValue)
  }
  await writeFile(reportPath, serializedReport, "utf8")
  if (failure) throw failure
  console.log(`[computer-clipboard-x11-drill] PASS evidence=${evidenceRoot}`)
}

main().catch((error) => {
  console.error(`[computer-clipboard-x11-drill] ${error.stack ?? error}`)
  process.exitCode ??= interruptedSignal === "SIGINT" ? 130 : interruptedSignal === "SIGTERM" ? 143 : 1
})
