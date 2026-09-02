#!/usr/bin/env node

import { spawn } from "node:child_process"
import { randomUUID } from "node:crypto"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const screenTool = path.join(
  repoRoot,
  "apps",
  "kernel",
  "slice-linux-docker",
  "docker",
  "slice-screen.sh",
)
const cargoTargetDir = path.resolve(
  process.env.CARGO_TARGET_DIR ??
    path.join(os.homedir(), ".chariox", "dev", "browser-computer-use", "cargo-target"),
)
const evidenceRoot = path.join(
  os.homedir(),
  ".codex",
  "evidence",
  "browser-computer-use",
  "computer-secret-input",
  `${new Date().toISOString().replaceAll(":", "-")}-${process.pid}`,
)

function run(command, args, { env = process.env, input = null } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString()
    })
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString()
    })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      resolve({ command, args, code, signal, stdout, stderr })
    })
    if (input == null) child.stdin.end()
    else child.stdin.end(input)
  })
}

async function writeExecutable(file, source) {
  await writeFile(file, source, "utf8")
  await chmod(file, 0o755)
}

function requirePass(result, label, secret = null) {
  if (result.code === 0) return
  const output = `${result.stdout}\n${result.stderr}`
  const safeOutput = secret == null ? output : output.replaceAll(secret, "[redacted]")
  throw new Error(`${label} failed with exit ${result.code}${result.signal ? ` (${result.signal})` : ""}\n${safeOutput}`)
}

async function verifyDesktopHelper() {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-computer-secret-drill-"))
  const fakeBin = path.join(root, "bin")
  const argsPath = path.join(root, "xdotool-args")
  const inputPath = path.join(root, "xdotool-input")
  const clipboardSentinel = path.join(root, "clipboard-used")
  const nodeSentinel = path.join(root, "node-used")
  const secret = `computer-secret-drill-${randomUUID()}`
  try {
    await mkdir(fakeBin, { recursive: true })
    await writeExecutable(path.join(fakeBin, "xdpyinfo"), "#!/bin/sh\nexit 0\n")
    await writeExecutable(
      path.join(fakeBin, "pgrep"),
      "#!/bin/sh\nprintf '123 drill-process\\n'\n",
    )
    await writeExecutable(
      path.join(fakeBin, "xdotool"),
      "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" > \"$CHARIOX_DRILL_XDOTOOL_ARGS\"\ncat > \"$CHARIOX_DRILL_XDOTOOL_INPUT\"\n",
    )
    await writeExecutable(
      path.join(fakeBin, "timeout"),
      "#!/bin/sh\nset -eu\nshift\nexec \"$@\"\n",
    )
    await writeExecutable(
      path.join(fakeBin, "xclip"),
      "#!/bin/sh\n: > \"$CHARIOX_DRILL_CLIPBOARD_SENTINEL\"\nexit 97\n",
    )
    await writeExecutable(
      path.join(fakeBin, "node"),
      "#!/bin/sh\n: > \"$CHARIOX_DRILL_NODE_SENTINEL\"\nexit 98\n",
    )
    const result = await run("/bin/bash", [screenTool, "computer-secret-paste-stdin"], {
      input: secret,
      env: {
        ...process.env,
        PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
        HOME: path.join(root, "home"),
        CHARIOX_SLICE_ROOT: path.join(root, "slice"),
        CHARIOX_SLICE_CHROME_PROFILE: path.join(root, "chromium-profile"),
        CHARIOX_DRILL_XDOTOOL_ARGS: argsPath,
        CHARIOX_DRILL_XDOTOOL_INPUT: inputPath,
        CHARIOX_DRILL_CLIPBOARD_SENTINEL: clipboardSentinel,
        CHARIOX_DRILL_NODE_SENTINEL: nodeSentinel,
      },
    })
    requirePass(result, "desktop helper", secret)
    if (`${result.stdout}${result.stderr}`.includes(secret)) {
      throw new Error("desktop helper emitted the secret")
    }
    const argumentsUsed = (await readFile(argsPath, "utf8")).trim()
    if (argumentsUsed !== "type --clearmodifiers --delay 5 --file -") {
      throw new Error(`desktop helper used unexpected xdotool arguments: ${argumentsUsed}`)
    }
    if ((await readFile(inputPath, "utf8")) !== secret) {
      throw new Error("desktop helper did not preserve the exact secret bytes")
    }
    for (const [sentinel, label] of [
      [clipboardSentinel, "clipboard"],
      [nodeSentinel, "browser controller"],
    ]) {
      try {
        await readFile(sentinel)
        throw new Error(`desktop helper unexpectedly invoked the ${label}`)
      } catch (error) {
        if (error?.code !== "ENOENT") throw error
      }
    }
    return {
      name: "desktop-helper-existing-focus",
      status: "passed",
      xdotoolArguments: argumentsUsed,
      clipboardUsed: false,
      browserControllerUsed: false,
      secretEmitted: false,
    }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

async function main() {
  await mkdir(evidenceRoot, { recursive: true })
  const report = {
    schema: "chariox.computer_secret_input_drill.v1",
    startedAt: new Date().toISOString(),
    repoRoot,
    cargoTargetDir,
    checks: [],
  }
  try {
    const syntax = await run("/bin/bash", ["-n", screenTool])
    requirePass(syntax, "slice-screen syntax")
    report.checks.push({ name: "slice-screen-syntax", status: "passed" })
    report.checks.push(await verifyDesktopHelper())

    const filters = [
      "computer_secret_input",
      "credential_specs_expose_browser_and_computer_secret_paste_tools",
      "mcp_tools_call_dispatches_slice_screen_fallbacks_inside_slice_kernel",
      "relay_home_credential_proxy_shape_is_versioned",
      "relay_home_credential_secret_debug_output_is_redacted",
    ]
    for (const filter of filters) {
      const result = await run(
        "cargo",
        ["test", "-p", "chariox-kernel", "--lib", filter, "--", "--nocapture"],
        {
          env: {
            ...process.env,
            CARGO_BUILD_JOBS: "1",
            CARGO_TARGET_DIR: cargoTargetDir,
          },
        },
      )
      requirePass(result, `cargo test ${filter}`)
      await writeFile(
        path.join(evidenceRoot, `${filter}.log`),
        `${result.stdout}${result.stderr}`,
        "utf8",
      )
      report.checks.push({ name: `cargo-test:${filter}`, status: "passed" })
    }
    report.status = "passed"
    report.completedAt = new Date().toISOString()
    await writeFile(path.join(evidenceRoot, "report.json"), `${JSON.stringify(report, null, 2)}\n`)
    console.log(`[computer-secret-input-drill] PASS evidence=${evidenceRoot}`)
  } catch (error) {
    report.status = "failed"
    report.completedAt = new Date().toISOString()
    report.error = error instanceof Error ? error.message : String(error)
    await writeFile(path.join(evidenceRoot, "report.json"), `${JSON.stringify(report, null, 2)}\n`)
    throw error
  }
}

main().catch((error) => {
  console.error(`[computer-secret-input-drill] ${error.stack ?? error}`)
  process.exitCode = 1
})
