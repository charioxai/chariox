// FA06 / A12-S05: one real 60-minute /sudo window on a real kernel with a real Claude agent, driven by the owner's passkey.
// The agent probes a privileged kernel request (chariox kernel_request against a second Room) at 0, 30, 60, 90 and 120
// minutes. The driver waits for the 10-minute warning, extends with a fresh passkey (new expiry = verification + 60 min),
// and lets the extended window lapse. Expected: probes 1-4 answer, probe 5 is denied, the warning shows once per window
// revision, and the lapsed window ends with a regular re-evaluation of the waiting work.
// usage: node scripts/drills/sudo-window.mjs --kernel-binary <path> --state-root /root/.chariox/dev/<lane>/... \
//   --evidence-dir <dir> --claude-profile-path <product-linked Claude profile directory>
import { randomBytes } from "node:crypto"
import { writeFileSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { submitPromptRequest } from "../../packages/kernel-client/dist/ipc-requests.js"
import { respondToInteractionRequest } from "../../packages/kernel-client/dist/ipc-terminal-runtime-requests.js"
import { drillFlags, startDrillKernel, until } from "./own-kernel.mjs"

const { flags, need } = drillFlags(process.argv)
const evidence = need("--evidence-dir")
const model = flags.get("--model") ?? "claude-haiku-5-5"
const probeMinutes = Number(flags.get("--probe-minutes") ?? 30)
const passkey = randomBytes(24).toString("hex")
const k = await startDrillKernel({ label: "sudowin", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence, profilePath: need("--claude-profile-path") })
let exitCode = 0
const expect = (name, ok, detail = {}) => { k.note(`expect:${name}`, { ok: Boolean(ok), ...detail }); if (!ok) exitCode = 1 }
try {
  const { R, agent, attached } = await k.room("sudowin-room", model)
  const other = await k.room("sudowin-other", model)
  const pending = k.rpc({ ManageCredentialVault: { session_id: R, agent_id: agent } })
  pending.catch(() => {})
  const unlock = await until(async () => (await k.session(R)).active_interactions?.find(x => x.id.startsWith("vault-unlock-")), "vault prompt")
  await k.rpc({ RespondToInteraction: { session_id: R, interaction_id: unlock.id, choice_id: "passphrase", custom_reply: passkey } })
  await pending
  k.note("passkey-set")
  const answers = async () => (await k.rpc({ GetSessionHistoryOutline: { session_id: R, agent_ids: [agent], latest_prompt_count: 60, include_prefetched_content: true } })).SessionHistoryOutline.agents[0]?.turns ?? []
  const probeFor = n => (all => all.filter(turn => JSON.stringify(turn).includes(`PROBE-${n} `) && !`${turn.user_prompt?.entry?.text ?? ""}`.startsWith("/sudo")))
  const windows = async () => (await k.session(R)).sudo_windows ?? []
  const work = `/sudo You are probing a privileged kernel request over two hours. Probe n means: call the chariox kernel_request tool with request {"GetSessionState":{"session_id":"${other.R}"}} and reply with one line "PROBE-n OK" followed by that Room's alias, or "PROBE-n DENIED" followed by the exact reason if the call is refused. Do probe 1 now. After each probe arm a chariox events_timer of ${probeMinutes} minutes (delay_ms ${probeMinutes * 60000}) labelled PROBE-<next n>, call events_yield on it and end your turn; on its wake do the next probe. Stop after probe 5 and reply SUDO_DRILL_DONE.`
  await k.rpc(submitPromptRequest(R, attached, agent, work, []))
  const card = await until(async () => (await k.session(R)).active_interactions?.find(x => x.choices?.some(c => c.requires_passkey) && !x.id.startsWith("vault-")), "sudo passkey card", 120000)
  k.note("sudo-card", { id: card.id, title: card.title })
  const approved = Date.now()
  await k.rpc(respondToInteractionRequest(R, card.id, "approve", null, { passkey }))
  const first = (await until(async () => (await windows())[0], "window", 60000))
  k.note("window-open", { entry: first.entry_id, revision: first.revision, minutes: first.duration_minutes, expires_at_ms: first.expires_at_ms })
  expect("window-is-60-minutes", first.duration_minutes === 60)
  await until(async () => probeFor(1)(await answers()).length > 0, "probe 1", 240000)
  const warned = await until(async () => (await windows()).find(w => w.warning_sent), "10-minute warning", 55 * 60000)
  k.note("warning", { entry: warned.entry_id, revision: warned.revision, left_s: Math.round((warned.expires_at_ms - Date.now()) / 1000) })
  await k.rpc({ ExtendKernelSudo: { session_id: R, attachment_id: attached, entry_id: warned.entry_id, revision: warned.revision } }).catch(e => k.note("extend-request-ended", { message: String(e.message).slice(0, 200) }))
  const extendCard = await until(async () => (await k.session(R)).active_interactions?.find(x => /Extend sudo window/.test(x.title ?? "")), "extend card", 120000)
  await k.rpc(respondToInteractionRequest(R, extendCard.id, "approve", null, { passkey }))
  const extended = await until(async () => (await windows()).find(w => w.revision > warned.revision), "extended revision", 60000)
  k.note("extended", { revision: extended.revision, expires_at_ms: extended.expires_at_ms, warning_sent: extended.warning_sent, minutes_from_now: Math.round((extended.expires_at_ms - Date.now()) / 60000) })
  expect("extend-resets-warning", extended.warning_sent === false)
  expect("extend-is-verification-plus-60", Math.abs(extended.expires_at_ms - (Date.now() + 60 * 60000)) < 5 * 60000)
  for (const n of [2, 3, 4]) {
    await until(async () => probeFor(n)(await answers()).length > 0, `probe ${n}`, (probeMinutes + 15) * 60000)
    const text = JSON.stringify(probeFor(n)(await answers()).at(-1) ?? "")
    k.note(`probe-${n}`, { ok: text.includes(`PROBE-${n} OK`), denied: text.includes(`PROBE-${n} DENIED`) })
    expect(`probe-${n}-answered`, text.includes(`PROBE-${n} OK`))
  }
  await until(async () => (await windows()).length === 0, "window ends", 80 * 60000)
  k.note("window-ended", { after_min: Math.round((Date.now() - approved) / 60000) })
  await until(async () => probeFor(5)(await answers()).length > 0, "probe 5", 40 * 60000)
  const last = JSON.stringify(probeFor(5)(await answers()).at(-1) ?? "")
  expect("probe-5-denied-after-expiry", last.includes("PROBE-5 DENIED"), { text: last.slice(0, 300) })
  writeFileSync(join(evidence, "turns.json"), JSON.stringify(await answers(), null, 1), { mode: 0o600 })
} catch (error) {
  k.note("error", { message: String(error.message ?? error).slice(0, 300) })
  exitCode = 1
} finally {
  await k.stop()
}
process.exit(exitCode)
