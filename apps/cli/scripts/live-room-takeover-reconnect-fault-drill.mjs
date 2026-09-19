#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  ROOM_TAKEOVER_RECONNECT_CASE_IDS,
  ROOM_TAKEOVER_RECONNECT_TEST_NAME,
  buildRoomTakeoverReconnectCargoArgs,
  parseRoomTakeoverReconnectProbe,
} from "./lib/room-takeover-reconnect-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const runnerContext = readOptionalRunnerContext()

if (runnerContext) {
  // The Rust probe owns a standalone kernel/test authority today. Never let
  // Path 1 silently run that probe against a second session: the adapter must
  // fail closed until the probe accepts the supplied Room context.
  console.error("[room-takeover-reconnect-fault-drill] runner-context mode requires a context-capable kernel fault probe")
  process.exitCode = 2
} else {
  runLocalRustFaultDrill({
    argv: process.argv.slice(2),
    repoRoot,
    name: "live-room-takeover-reconnect-fault-drill.mjs",
    description: "Runs the exact kernel probe for human input authority across a lost takeover response and reconnect.",
    schema: "chariox.room_takeover_reconnect_fault_drill.v1",
    caseIds: ROOM_TAKEOVER_RECONNECT_CASE_IDS,
    cargoArgs: buildRoomTakeoverReconnectCargoArgs(),
    parseProbe: parseRoomTakeoverReconnectProbe,
    evidenceSubdir: "room-takeover-reconnect",
    processNeedle: ROOM_TAKEOVER_RECONNECT_TEST_NAME,
  }).catch((error) => {
    console.error(`[room-takeover-reconnect-fault-drill] ${bounded(error.stack ?? error.message)}`)
    process.exitCode = 1
  })
}

function readOptionalRunnerContext() {
  if (!process.argv.includes("--path1-runner-context")) return null
  const raw = process.env.CHARIOX_PATH1_RUNNER_CONTEXT_JSON
  if (!raw) throw new Error("--path1-runner-context requires CHARIOX_PATH1_RUNNER_CONTEXT_JSON")
  let context
  try { context = JSON.parse(raw) } catch { throw new Error("Path 1 runner context is not valid JSON") }
  if (context?.schema !== "chariox.path1.runner-context.v1") throw new Error("Path 1 runner context schema is unsupported")
  if (!context.sessionId || !context.roomId || !context.kernelEndpoint || !context.relayEndpoint) {
    throw new Error("Path 1 takeover context must name an existing kernel session, Room, and relay")
  }
  return context
}
