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
