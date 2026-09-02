#!/usr/bin/env node

import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { createHash, randomUUID } from "node:crypto"
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

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
const containerName = `chariox-computer-secret-x11-${process.pid}`
const evidenceRoot = path.join(
  os.homedir(),
  ".codex",
  "evidence",
  "browser-computer-use",
  "computer-secret-input-x11",
  runId,
)

const children = new Set()
const commandOutput = []
let containerCreated = false
let secretLeakDetected = false

function redact(value, secret) {
  return secret == null ? value : value.replaceAll(secret, "[redacted]")
}

function run(command, args, { input = null, timeoutMs = 120_000, secret = null } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env: process.env,
      stdio: ["pipe", "pipe", "pipe"],
    })
    children.add(child)
    let stdout = ""
    let stderr = ""
    const timeout = setTimeout(() => {
      child.kill("SIGTERM")
      setTimeout(() => child.kill("SIGKILL"), 2_000).unref()
    }, timeoutMs)
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString()
    })
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString()
    })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      children.delete(child)
      const result = { command, args, code, signal, stdout, stderr }
      const output = `${stdout}${stderr}`
      if (secret != null && output.includes(secret)) secretLeakDetected = true
      commandOutput.push(redact(output, secret))
      resolve(result)
    })
    if (input == null) child.stdin.end()
    else child.stdin.end(input)
  })
}

async function requireRun(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    const output = redact(`${result.stdout}${result.stderr}`, options.secret)
    throw new Error(
      `${command} ${args.join(" ")} failed with exit ${result.code}${
        result.signal ? ` (${result.signal})` : ""
      }\n${output}`,
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

function testPage(expectedDigest, expectedLength) {
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>BOOTING</title>
    <style>
      html, body { height: 100%; }
      body {
        align-items: center;
        background: #f4f4f3;
        color: #202124;
        display: flex;
        font: 24px system-ui, sans-serif;
        justify-content: center;
        margin: 0;
      }
      main { width: 680px; }
      label { display: block; font-weight: 650; margin-bottom: 14px; }
      input {
        background: white;
        border: 3px solid #333;
        border-radius: 10px;
        box-sizing: border-box;
        font: 34px system-ui, sans-serif;
        padding: 14px 18px;
        width: 100%;
      }
      #status { font-weight: 650; margin-top: 18px; }
    </style>
  </head>
  <body>
    <main>
      <label for="secret">MASKED COMPUTER INPUT</label>
      <input id="secret" type="password" autocomplete="off" autofocus>
      <div id="status">MASKED INPUT READY</div>
    </main>
    <script>
      const expectedDigest = ${JSON.stringify(expectedDigest)};
      const expectedLength = ${expectedLength};
      const input = document.getElementById("secret");
      const status = document.getElementById("status");
      async function verify() {
        const bytes = new TextEncoder().encode(input.value);
        const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)))
          .map((value) => value.toString(16).padStart(2, "0"))
          .join("");
        const passed = input.value.length === expectedLength && digest === expectedDigest;
        document.title = (passed ? "PASS" : "WAIT") + ":" + input.value.length;
        status.textContent = passed ? "EXACT MASKED INPUT RECEIVED" : "MASKED INPUT READY";
      }
      input.addEventListener("input", verify);
      window.addEventListener("load", () => {
        input.focus();
        document.title = "READY";
      });
    </script>
  </body>
</html>
`
}

async function title() {
  const result = await docker(
    [
      "exec",
      "-u",
      "slice",
      "-e",
      "DISPLAY=:99",
      containerName,
      "/bin/bash",
      "-lc",
      "window=$(xdotool search --onlyvisible --name '^(READY|WAIT|PASS|BOOTING)' 2>/dev/null | grep -E '^[1-9][0-9]*$' | head -n 1); test -n \"$window\"; xdotool getwindowname \"$window\"",
    ],
    { timeoutMs: 10_000 },
  )
  return result.stdout.trim()
}

async function focusPasswordField() {
  await docker([
    "exec",
    "-u",
    "slice",
    "-e",
    "DISPLAY=:99",
    containerName,
    "/bin/bash",
    "-lc",
    "window=$(xdotool search --onlyvisible --name '^(READY|WAIT|PASS)' 2>/dev/null | grep -E '^[1-9][0-9]*$' | head -n 1); test -n \"$window\"; xdotool windowactivate --sync \"$window\"; xdotool windowfocus --sync \"$window\"; xdotool mousemove --window \"$window\" 640 430 click 1",
  ])
}

async function collectDiagnostics(secret) {
  if (!containerCreated) return "container was not created"
  const result = await run(
    "docker",
    [
      "exec",
      "-u",
      "slice",
      "-e",
      "DISPLAY=:99",
      containerName,
      "/bin/bash",
      "-lc",
      "printf '%s\\n' '[processes]'; ps -eo pid,rss,stat,args; printf '%s\\n' '[windows]'; xwininfo -root -tree 2>&1 || true; for log in /tmp/chariox-secret-x11/*.log; do printf '[log %s]\\n' \"$log\"; tail -n 80 \"$log\"; done",
    ],
    { timeoutMs: 20_000, secret },
  )
  return redact(`${result.stdout}${result.stderr}`, secret)
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
  if (containerCreated) {
    await run("docker", ["rm", "-f", containerName], { timeoutMs: 30_000 }).catch(() => null)
    containerCreated = false
  }
  await rm(tempRoot, { recursive: true, force: true })
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

async function main() {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-computer-secret-x11-"))
  const secret = `cxs-${randomUUID().replaceAll("-", "")}-Z9`
  const expectedDigest = createHash("sha256").update(secret).digest("hex")
  const pagePath = path.join(tempRoot, "index.html")
  const screenshotPath = path.join(evidenceRoot, "masked-field.png")
  const ocrPath = path.join(evidenceRoot, "masked-field-ocr.txt")
  const reportPath = path.join(evidenceRoot, "report.json")
  const resources = []
  const sourceCommit = await requireRun("git", ["rev-parse", "HEAD"], { timeoutMs: 10_000 })
  const report = {
    schema: "chariox.computer_secret_input_x11_drill.v1",
    startedAt: new Date().toISOString(),
    command: "pnpm --dir apps/cli computer-secret-input:x11-drill",
    sourceCommits: {
      oss: sourceCommit.stdout.trim(),
      cloud: null,
    },
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
      kernel: "not exercised; covered by the companion protocol drill",
      relay: "not exercised",
      cloud: "not exercised",
      selkies: "not exercised",
    },
    artifacts: {
      screenshot: "masked-field.png",
      ocr: "masked-field-ocr.txt",
    },
    resourceLimits: {
      memoryBytes: 805_306_368,
      memorySwapBytes: 805_306_368,
      nanoCpus: 1_000_000_000,
      pidsLimit: 256,
    },
    checks: [],
    resources,
  }

  await mkdir(evidenceRoot, { recursive: true })
  await writeFile(pagePath, testPage(expectedDigest, secret.length), "utf8")
  resources.push(await resourceSnapshot("before"))

  let failure = null
  try {
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
    const inspect = await docker([
      "container",
      "inspect",
      "--format",
      "{{json .HostConfig}}",
      containerName,
    ])
    const hostConfig = JSON.parse(inspect.stdout)
    assert.equal(hostConfig.Memory, report.resourceLimits.memoryBytes)
    assert.equal(hostConfig.MemorySwap, report.resourceLimits.memorySwapBytes)
    assert.equal(hostConfig.NanoCpus, report.resourceLimits.nanoCpus)
    assert.equal(hostConfig.PidsLimit, report.resourceLimits.pidsLimit)
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
      ". /etc/os-release; printf 'os=%s-%s\\n' \"$ID\" \"$VERSION_ID\"; chromium --version",
    ])
    const [containerOs, browser] = containerVersions.stdout.trim().split("\n")
    report.components.containerOs = containerOs
    report.components.browser = browser
    await docker(["exec", "-u", "root", containerName, "mkdir", "-p", "/tmp/chariox-secret-x11"])
    await docker(["cp", pagePath, `${containerName}:/tmp/chariox-secret-x11/index.html`])
    await docker(["cp", screenTool, `${containerName}:/tmp/chariox-secret-x11/slice-screen.sh`])
    await docker([
      "exec",
      "-u",
      "root",
      containerName,
      "chown",
      "-R",
      "slice:slice",
      "/tmp/chariox-secret-x11",
    ])

    await docker([
      "exec",
      "-d",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      "Xvfb :99 -screen 0 1280x800x24 -ac +extension RANDR +extension XTEST >/tmp/chariox-secret-x11/xvfb.log 2>&1",
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
      "openbox >/tmp/chariox-secret-x11/openbox.log 2>&1",
    ])
    await docker([
      "exec",
      "-d",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      "python3 -m http.server 8765 --bind 127.0.0.1 --directory /tmp/chariox-secret-x11 >/tmp/chariox-secret-x11/http.log 2>&1",
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
      "exec chromium --user-data-dir=/tmp/chariox-secret-x11/profile --no-sandbox --password-store=basic --no-first-run --no-default-browser-check --disable-sync --disable-dev-shm-usage --disable-gpu --disable-background-networking --window-size=1280,800 http://127.0.0.1:8765/ >/tmp/chariox-secret-x11/chromium.log 2>&1",
    ])

    await waitFor(async () => {
      const current = await title()
      return current.startsWith("READY") ? current : false
    }, "password page readiness", 45_000)
    await focusPasswordField()

    const clipboardSentinel = `clipboard-sentinel-${runId}`
    await docker(
      [
        "exec",
        "-u",
        "slice",
        "-e",
        "DISPLAY=:99",
        "-e",
        "CHARIOX_SLICE_CHROME_PROFILE=/tmp/chariox-secret-x11/profile",
        containerName,
        "/tmp/chariox-secret-x11/slice-screen.sh",
        "clipboard-set",
        clipboardSentinel,
      ],
      { timeoutMs: 10_000 },
    )

    const inputResult = await docker(
      [
        "exec",
        "-i",
        "-u",
        "slice",
        "-e",
        "DISPLAY=:99",
        "-e",
        "CHARIOX_SLICE_CHROME_PROFILE=/tmp/chariox-secret-x11/profile",
        containerName,
        "/tmp/chariox-secret-x11/slice-screen.sh",
        "computer-secret-paste-stdin",
      ],
      { input: secret, secret, timeoutMs: 20_000 },
    )
    assert.equal(inputResult.stdout, "", "computer secret helper must not emit stdout")
    assert.equal(inputResult.stderr, "", "computer secret helper must not emit stderr")

    const verifiedTitle = await waitFor(
      async () => {
        const current = await title()
        return current.startsWith(`PASS:${secret.length}`) ? current : false
      },
      "exact X11 secret input",
    )
    assert.equal(verifiedTitle.startsWith(`PASS:${secret.length}`), true)

    const clipboard = await docker(
      [
        "exec",
        "-u",
        "slice",
        "-e",
        "DISPLAY=:99",
        "-e",
        "CHARIOX_SLICE_CHROME_PROFILE=/tmp/chariox-secret-x11/profile",
        containerName,
        "/tmp/chariox-secret-x11/slice-screen.sh",
        "clipboard-get",
      ],
      { timeoutMs: 10_000 },
    )
    assert.equal(clipboard.stdout, clipboardSentinel)

    await docker([
      "exec",
      "-u",
      "slice",
      "-e",
      "DISPLAY=:99",
      "-e",
      "CHARIOX_SLICE_CHROME_PROFILE=/tmp/chariox-secret-x11/profile",
      containerName,
      "/tmp/chariox-secret-x11/slice-screen.sh",
      "screenshot",
      "/tmp/chariox-secret-x11/masked-field.png",
    ])
    await docker(["cp", `${containerName}:/tmp/chariox-secret-x11/masked-field.png`, screenshotPath])
    const screenshot = await readFile(screenshotPath)
    assert.equal(screenshot.includes(Buffer.from(secret)), false, "screenshot bytes leaked the secret")

    const ocr = await docker([
      "exec",
      "-u",
      "slice",
      containerName,
      "tesseract",
      "/tmp/chariox-secret-x11/masked-field.png",
      "stdout",
    ])
    assert.match(ocr.stdout, /EXACT MASKED INPUT RECEIVED/)
    assert.equal(ocr.stdout.includes(secret), false, "screenshot OCR leaked the secret")
    await writeFile(ocrPath, ocr.stdout, "utf8")

    const controller = await docker([
      "exec",
      "-u",
      "slice",
      containerName,
      "/bin/bash",
      "-lc",
      "pgrep -af 'browser-controller|browser-cdp' | grep -v 'pgrep -af' || true",
    ])
    assert.equal(controller.stdout.trim(), "", "browser controller participated in Computer input")

    const logs = await docker(
      [
        "exec",
        "-i",
        "-u",
        "slice",
        containerName,
        "/bin/bash",
        "-lc",
        "needle=$(cat); find /tmp/chariox-secret-x11 -maxdepth 1 -type f -name '*.log' -exec grep -a -l -F -- \"$needle\" {} + || true",
      ],
      { input: secret, secret },
    )
    assert.equal(logs.stdout.trim(), "", "slice logs leaked the secret")

    const combinedOutput = commandOutput.join("\n")
    assert.equal(combinedOutput.includes(secret), false, "captured helper output leaked the secret")
    assert.equal(secretLeakDetected, false, "a secret-bearing command emitted the secret")

    report.checks.push(
      { name: "existing-slice-image", status: "passed" },
      { name: "resource-bounded-container", status: "passed" },
      { name: "real-x11-password-focus", status: "passed" },
      { name: "exact-secret-input-by-digest", status: "passed" },
      { name: "helper-output-redaction", status: "passed" },
      { name: "clipboard-unchanged", status: "passed" },
      { name: "masked-screenshot-and-ocr", status: "passed" },
      { name: "browser-controller-not-used", status: "passed" },
      { name: "slice-log-leak-scan", status: "passed" },
    )
    resources.push(await resourceSnapshot("during"))
    report.status = "passed"
  } catch (error) {
    report.status = "failed"
    report.error = redact(error instanceof Error ? error.message : String(error), secret)
    report.diagnostics = await collectDiagnostics(secret).catch((diagnosticError) =>
      redact(diagnosticError instanceof Error ? diagnosticError.message : String(diagnosticError), secret),
    )
    failure = error
  }

  await cleanup(tempRoot)
  resources.push(await resourceSnapshot("after"))
  const leftovers = await run("docker", [
    "ps",
    "-a",
    "--filter",
    `name=^/${containerName}$`,
    "--format",
    "{{.Names}}",
  ])
  const containerRemoved = leftovers.stdout.trim() === ""
  const tempRootRemoved = await pathMissing(tempRoot)
  report.cleanup = { containerRemoved, tempRootRemoved }
  if (!containerRemoved || !tempRootRemoved) {
    const cleanupError = new Error("drill cleanup left a container or temporary directory")
    report.status = "failed"
    report.error ??= cleanupError.message
    failure ??= cleanupError
  }
  report.completedAt = new Date().toISOString()
  const serializedReport = `${JSON.stringify(report, null, 2)}\n`
  const safeReport = redact(serializedReport, secret)
  assert.equal(safeReport.includes(secret), false, "retained report leaked the secret")
  await writeFile(reportPath, safeReport, "utf8")
  if (failure) throw failure
  console.log(`[computer-secret-input-x11-drill] PASS evidence=${evidenceRoot}`)
}

main().catch((error) => {
  console.error(`[computer-secret-input-x11-drill] ${error.stack ?? error}`)
  process.exitCode = 1
})
