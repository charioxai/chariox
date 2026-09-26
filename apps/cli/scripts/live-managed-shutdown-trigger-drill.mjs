#!/usr/bin/env node

import { realpathSync } from "node:fs"
import { createInterface } from "node:readline/promises"
import { stdin, stderr, stdout } from "node:process"
import { fileURLToPath } from "node:url"

import { validateCaptureOutput } from "./path1-provider-rebuild-capture.mjs"
import { loadManagedShutdownKernelClient } from "./lib/managed-shutdown-trigger-kernel-client.mjs"
import { parseArguments } from "./lib/managed-shutdown-trigger-config.mjs"
import { runManagedShutdownTrigger } from "./lib/managed-shutdown-trigger-capture.mjs"

export {
  SHUTDOWN_SCENARIOS,
  SHUTDOWN_TRIGGER_SCHEMA,
  parseArguments,
} from "./lib/managed-shutdown-trigger-config.mjs"
export { runManagedShutdownTrigger } from "./lib/managed-shutdown-trigger-capture.mjs"

function abortable(promise, signal) {
  if (!signal) return promise
  if (signal.aborted) {
    void Promise.resolve(promise).catch(() => {})
    return Promise.reject(new Error("capture interrupted"))
  }
  return new Promise((resolvePromise, rejectPromise) => {
    const cleanup = () => signal.removeEventListener("abort", onAbort)
    const onAbort = () => {
      cleanup()
      rejectPromise(new Error("capture interrupted"))
    }
    signal.addEventListener("abort", onAbort, { once: true })
    Promise.resolve(promise).then(
      (value) => { cleanup(); resolvePromise(value) },
      (error) => { cleanup(); rejectPromise(error) },
    )
  })
}

function defaultPrompt(message, signal) {
  const prompt = createInterface({ input: stdin, output: stderr })
  return abortable(prompt.question(`${message}\nPress Enter after the requested product action is complete. `), signal)
    .finally(() => prompt.close())
}

async function main(argv = process.argv.slice(2)) {
  const options = parseArguments(argv)
  await validateCaptureOutput(options.output)
  if (["agents_done", "idle_stop", "minimum_runtime", "disabled", "keep_running",
    "restart_reconciliation", "deployment_reconciliation", "manual"]
    .includes(options.descriptor.mode)) {
    if (!stdin.isTTY) throw new Error("this scenario requires an interactive owner")
  }
  const controller = new AbortController()
  const onSignal = () => controller.abort()
  process.once("SIGINT", onSignal)
  process.once("SIGTERM", onSignal)
  try {
    const { client, requests } = await loadManagedShutdownKernelClient(options.kernelUrl)
    await runManagedShutdownTrigger(options, {
      client,
      requests,
      signal: controller.signal,
      ask: (message, promptSignal) => defaultPrompt(message, promptSignal),
    })
    stdout.write("Managed shutdown observations written; no acceptance verdict.\n")
  } catch {
    stderr.write("Managed shutdown capture incomplete; no acceptance verdict.\n")
    process.exitCode = 1
  } finally {
    process.removeListener("SIGINT", onSignal)
    process.removeListener("SIGTERM", onSignal)
  }
}

function invokedDirectly() {
  if (!process.argv[1]) return false
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
  } catch { return false }
}

if (invokedDirectly()) {
  main().catch(() => {
    stderr.write("Managed shutdown capture failed; no acceptance verdict.\n")
    process.exitCode = 1
  })
}
