#!/usr/bin/env node
import { spawnSync } from "node:child_process"

const drills = [
  {
    name: "kernel performance drills",
    command: "cargo",
    args: [
      "test",
      "--manifest-path",
      "apps/kernel/Cargo.toml",
      "performance_drill",
      "--",
      "--nocapture",
    ],
  },
  {
    name: "relay performance drills",
    command: "cargo",
    args: [
      "test",
      "--manifest-path",
      "apps/relay/Cargo.toml",
      "performance_drill",
      "--",
      "--nocapture",
    ],
  },
]

for (const drill of drills) {
  console.log(`\n== ${drill.name} ==`)
  const result = spawnSync(drill.command, drill.args, {
    stdio: "inherit",
  })
  if (result.error) {
    console.error(`${drill.name} failed to start: ${result.error.message}`)
    process.exit(1)
  }
  if (result.status !== 0) {
    console.error(`${drill.name} failed with exit code ${result.status}`)
    process.exit(result.status ?? 1)
  }
}

console.log("\nperformance drills passed")
