// MP-09 G04: drop one acknowledgement of a lifecycle delivery on a real kernel with a real Claude agent.
// The agent arms a timer and yields; the drill writes the `drop-ack` fault (docs/AGENT_WAKE_DRILL.md) for the
// delivery kind under test, so the write commits and its caller is told it failed. The retry or the sweep must
// reconcile it: the wake reaches the agent once, and the person sees the outcome.
// usage: node scripts/drills/ack-drop.mjs --kernel-binary <path> --state-root /root/.chariox/dev/<lane>/... \
//   --evidence-dir <dir> --claude-profile-path <product-linked Claude profile directory> --kinds wake_intent,delivery_receipt
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { DEFAULT_KINDS, wakeTurns, wokeReply } from "./ack-drop-support.mjs"
import { drillFlags, startDrillKernel, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const evidence = need("--evidence-dir")
const kinds = flags.has("--kinds") ? flags.get("--kinds").split(",") : DEFAULT_KINDS
const model = flags.get("--model") ?? "claude-haiku-5-5"
const stateRoot = need("--state-root")
const faults = join(stateRoot, "faults")
mkdirSync(faults, { recursive: true, mode: 0o700 })
const k = await startDrillKernel({ label: "ackdrop", binary: need("--kernel-binary"), stateRoot, evidence, profilePath: need("--claude-profile-path"), extraEnv: { CHARIOX_AGENT_FAULT_DIR: faults } })
const lines = file => existsSync(file) ? readFileSync(file, "utf8").split("\n").filter(Boolean).length : 0
let exitCode = 0
try {
  const { R, agent, attached } = await k.room("ackdrop-room", model)
  k.note("room-ready", { room: R, agent })
  const turns = async () => (await k.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: [agent], latest_prompt_count: 40, include_prefetched_content: true } })).SessionHistoryOutline.agents[0]?.turns ?? []
  for (const kind of kinds) {
    const label = `ACKDROP-${kind.toUpperCase().replace(/_/g, "")}`
    const dropLog = join(faults, "drop-ack.log")
    const before = lines(dropLog)
    const earlierWakes = wakeTurns(await turns()).length
    writeFileSync(join(faults, "drop-ack"), `${kind}\n`)
    k.note("armed", { kind, label })
    const ask = `Call the chariox events_timer tool with delay_ms 60000 and label "${label}". Then call chariox events_yield on that registration and end your turn. When the timer wake arrives reply with exactly ${label}_WOKE and nothing else.`
    await k.rpc(submitPromptRequest(R, attached, agent, ask, []))
    const dropped = await until(() => lines(dropLog) > before, `${kind} acknowledgement dropped`, 240000).then(() => true, () => false)
    k.note("dropped", { kind, dropped })
    await delay(Number(flags.get("--settle-seconds") ?? 150) * 1000)
    const all = await turns()
    const wakePrompts = wakeTurns(all).slice(earlierWakes)
    const s = await k.session(R)
    k.note("result", { kind, dropped, wakePrompts: wakePrompts.length, replied: wokeReply(wakePrompts, `${label}_WOKE`), origins: wakePrompts.map(turn => turn.prompt_origin), lifecycles: wakePrompts.map(turn => turn.lifecycle), notices: JSON.stringify(s.notices ?? null).slice(0, 400) })
    writeFileSync(join(evidence, `turns-${kind}.json`), JSON.stringify(all, null, 1), { mode: 0o600 })
    if (!dropped || wakePrompts.length !== 1 || !wokeReply(wakePrompts, `${label}_WOKE`)) exitCode = 1
  }
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 300) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)
