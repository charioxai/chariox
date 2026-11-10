// Turns the result.json files of run.mjs into one markdown table per variant and a PASS/FAIL verdict
// against the bar (Miguel, MP-08..MP-11): transfers at >= 90 % of the link, small-request p95 <= RTT + 100 ms,
// the kernel-list renewal never fails, nothing disconnected, the serving kernel stays connected while its
// reader is frozen and the relay RSS stays within baseline + 16 MiB, concurrent transfers share fairly.
//   node report.mjs <evidence-dir> <label>
import { existsSync, readFileSync, readdirSync } from "node:fs"
import { join } from "node:path"

const [dir, label] = process.argv.slice(2)
const ms = v => (v === null || v === undefined ? "-" : `${Math.round(v)}`)
const pct = v => (v === null || v === undefined ? "-" : `${(v * 100).toFixed(0)}%`)
const rows = readdirSync(dir).filter(n => n.startsWith(`${label}-`) && existsSync(join(dir, n, "result.json"))).sort()
const out = []
for (const name of rows) {
  const r = JSON.parse(readFileSync(join(dir, name, "result.json"), "utf8"))
  const rtt = r.link.rttMs, bar = rtt + 100
  const fails = []
  const done = (r.transfers ?? []).filter(t => t.code === 0 && t.hashOk)
  if (r.fatal) fails.push(`harness error: ${r.fatal}`)
  const frozen = r.variant === "stalled"
  if (!frozen && (!r.transfers || done.length !== r.transfers.length)) fails.push(`${(r.transfers ?? []).length - done.length} transfer(s) failed`)
  if (!frozen && typeof r.fileShare === "number" && r.fileShare < 0.9) fails.push(`file bytes ${pct(r.fileShare)} of the wire < 90%`)
  if (!frozen && r.wireDownRate !== null && r.wireDownRate !== undefined && r.wireDownRate < 0.9) fails.push(`link use ${pct(r.wireDownRate)} < 90%`)
  if (!frozen && r.probes && r.probes.small.p95 > bar) fails.push(`small p95 ${ms(r.probes.small.p95)} ms > ${bar} ms`)
  if (!frozen && r.probes && r.probes.small.failed) fails.push(`${r.probes.small.failed} small probe(s) failed`)
  if (!frozen && r.probes && r.probes.list.failed) fails.push(`${r.probes.list.failed} kernel-list probe(s) failed`)
  if (r.finalConnected && !(r.finalConnected.worker && r.finalConnected.home)) fails.push("a kernel is not connected after the run")
  if (r.workerLinkConnected === false) fails.push("the serving kernel reported its relay link down during the run")
  if (r.workerDuringStall && !r.workerDuringStall.allConnected) fails.push("the serving kernel lost its relay link while the reader was frozen")
  if (r.relayRssKib && r.relayRssKib.max - r.relayRssKib.before > 16 * 1024) fails.push(`relay RSS grew ${Math.round((r.relayRssKib.max - r.relayRssKib.before) / 1024)} MiB`)
  if (r.variant === "three" && done.length === 3) {
    const t = done.map(x => x.elapsedMs)
    if (Math.max(...t) / Math.min(...t) > 1.5) fails.push(`unfair: slowest ${ms(Math.max(...t))} ms vs fastest ${ms(Math.min(...t))} ms`)
  }
  const dropped = (r.errors ?? []).filter(e => e.count).map(e => `${e.source}:${e.count}`)
  out.push(`### ${name} — ${fails.length ? "FAIL" : "PASS"}`, "")
  if (fails.length) out.push(`Fails: ${fails.join("; ")}`, "")
  out.push(`Link ${r.link.down}/${r.link.up} kbit/s down/up, RTT ${rtt} ms (jitter ${r.link.jitterMs}), file ${(r.sizeBytes / 1048576).toFixed(0)} MiB x ${(r.transfers ?? []).length}.`, "")
  if (r.probes) {
    out.push("| probe (ms) | n | failed | p50 | p95 | max |", "|---|---|---|---|---|---|")
    for (const [title, key] of [["small request idle", "idleSmall"], ["small request during bulk", "small"], ["kernel list idle", "idleList"], ["kernel list during bulk", "list"]]) {
      const p = r.probes[key]
      out.push(`| ${title} | ${p.n} | ${p.failed} | ${ms(p.p50)} | ${ms(p.p95)} | ${ms(p.max)} |`)
    }
    out.push("")
  }
  out.push("| transfer | elapsed s | ideal s (size / link) | payload rate vs link | ok |", "|---|---|---|---|---|")
  ;(r.transfers ?? []).forEach((t, i) => out.push(`| ${i + 1} | ${t.elapsedMs ? (t.elapsedMs / 1000).toFixed(1) : "-"} | ${(r.sizeBytes / (r.link.down * 125)).toFixed(1)} | ${pct(r.payloadRate?.[i])} | ${t.code === 0 && t.hashOk ? "yes" : `no (${(t.error ?? "").slice(0, 120)})`} |`))
  if (typeof r.fileShare === "number") out.push("", `File bytes over wire bytes (verified file bytes / bytes the relay put on the ${r.direction === "put" ? "uplink" : "downlink"}, probes' cost taken off): ${pct(r.fileShare)} (${r.fileBytes} B in ${r.wire.downBytes} B); the bar is 90%.`)
  out.push("", `Wire use of the ${r.direction === "put" ? "uplink" : "downlink"} by the transfers (bytes the relay sent through the shaped link / window / link rate, the probes' own idle cost taken off; raw ${pct(r.wireDownRateRaw)}): ${pct(r.wireDownRate)}; a probe pair costs ${r.wire?.probePairBytes ?? '-'} B. Window ${ms(r.windowMs / 1000)} s; the ideal for all bytes is ${r.idealSeconds ? r.idealSeconds.toFixed(0) : "-"} s of payload.`)
  if (r.cpuSeconds) out.push(`CPU seconds used during the run (debug builds): relay ${r.cpuSeconds.relay}, serving kernel ${r.cpuSeconds.worker}, client kernel ${r.cpuSeconds.home}.`)
  out.push(`Relay RSS KiB: before ${r.relayRssKib?.before}, max ${r.relayRssKib?.max}, after ${r.relayRssKib?.after}${r.relayRssKib?.duringStall ? `, during stall ${r.relayRssKib.duringStall}` : ""}.`)
  out.push(`Serving kernel presence lines in the log: ${r.presence?.lines} (max gap ${r.presence?.maxGapS === null || r.presence?.maxGapS === undefined ? "-" : r.presence.maxGapS.toFixed(0) + " s"}); link reported up all run: ${r.workerLinkConnected}; connected after: ${JSON.stringify(r.finalConnected)}.`)
  if (r.burst) out.push(`Burst of ${r.burst.sent} small requests queued for the frozen client: ${r.burst.answered ?? "not all"} answered after the thaw.`)
  if (r.workerDuringStall) out.push(`Frozen-reader window: ${r.workerDuringStall.samples} samples of the serving kernel, all connected: ${r.workerDuringStall.allConnected}, slowest ${ms(r.workerDuringStall.maxMs)} ms.`)
  if (r.probes?.smallErrors?.length || r.probes?.listErrors?.length) out.push(`Probe errors: small ${JSON.stringify(r.probes.smallErrors)}, list ${JSON.stringify(r.probes.listErrors)}.`)
  out.push(`Log lines matching disconnect/timeout/error words: ${dropped.join(", ") || "none"}.`)
  for (const e of r.errors ?? []) for (const line of e.first.slice(0, 2)) out.push(`  - ${e.source}: \`${line.slice(0, 200).replace(/`/g, "'")}\``)
  out.push("")
}
console.log(out.join("\n"))
