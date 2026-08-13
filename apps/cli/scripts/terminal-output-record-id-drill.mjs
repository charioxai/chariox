#!/usr/bin/env node
import { spawn } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")

const steps = [
  {
    name: "kernel stream record ids",
    command: "cargo",
    args: [
      "test",
      "--manifest-path",
      "apps/kernel/Cargo.toml",
      "terminal::stream::tests::output_polling_is_per_recipient",
      "--",
      "--exact",
    ],
  },
  {
    name: "terminal output protocol shape",
    command: "cargo",
    args: [
      "test",
      "--manifest-path",
      "apps/kernel/Cargo.toml",
      "local::api::tests::protocol_shapes::local_daemon_protocol_terminal_output_shape_is_versioned",
      "--",
      "--exact",
    ],
  },
  {
    name: "protocol version client conformance",
    command: "cargo",
    args: [
      "test",
      "--manifest-path",
      "apps/kernel/Cargo.toml",
      "local_daemon_protocol_version_matches_typescript_kernel_client",
      "--lib",
    ],
  },
  {
    name: "kernel client TypeScript build",
    command: "pnpm",
    args: ["--filter", "@chariox/kernel-client", "run", "build"],
  },
]

for (const step of steps) {
  console.log(`[terminal-output-record-id-drill] ${step.name}`)
  await run(step.command, step.args)
}

console.log("[terminal-output-record-id-drill] ok")

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      stdio: "inherit",
      env: process.env,
    })
    child.on("error", reject)
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}`))
    })
  })
}
