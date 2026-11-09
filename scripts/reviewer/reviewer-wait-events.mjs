#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11 (ON-17): waits for a workflow run to end by following the workflow's "run finished"
// notification source with the CLI, instead of listing its runs every minute. Same stdout contract as a polling wait:
//   <HH:MM>Z review <Completed|Failed> <repo> #<N> @<head10>[: <failure message>]    one line per run that ended
//   no review finished in the window                                                 nothing ended before the window closed
//   reviewer kernel unreachable                                                      the kernel did not answer
//   event follow ended: <reason>                                                     the grant was revoked or expired: request a new one
// It exits 0 in all of those cases and 1 only for a wrong command line or a missing CLI. Every run is printed once, in the
// order the kernel recorded the ends: the kernel keeps each notification until it is acknowledged, so a run that ends
// while no wait is running is printed by the next one.
// A missed notification cannot hide a run: when no notification arrives for --fallback-minutes (or the kernel reports a
// gap, i.e. unread notifications were dropped) the workflow's runs are listed once, like a poll would, and any run
// that ended since this wait started and was not printed yet is printed.
// The kernel and the grant come from the environment of the caller, as for every `chariox` command.
// Usage: reviewer-wait-events.mjs [--window-minutes 25] [--fallback-minutes 10] [--source NAME|ID]... [--chariox PATH]
//                                 [--state FILE]
import { spawn } from "node:child_process"
import { readFileSync, renameSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { pathToFileURL } from "node:url"

const KEEP_PRINTED = 500
// One `wait` never lasts longer than this, so a window or fallback deadline is noticed at once.
const MAX_WAIT_SECONDS = 60

export function formatLine({ at, status, prompt, runId, failure }) {
  const time = new Date(at).toISOString().slice(11, 16)
  const head = /for (\S+) #(\d+) at head ([0-9a-f]{7,40})/i.exec(prompt ?? "")
  const subject = head ? `${head[1]} #${head[2]} @${head[3].slice(0, 10)}` : `run ${runId}`
  return `${time}Z review ${status} ${subject}${status === "Failed" && failure ? `: ${failure}` : ""}`
}

// The one place that knows how the CLI reports a kernel that does not answer.
export function kernelUnreachable(result) {
  return result.code !== 0 && result.code !== 5 && /ECONNREFUSED|ECONNRESET|ENOENT|EPIPE|unreachable|could not connect|connection (closed|refused|reset)|socket|timed out|shutting down/i.test(result.stderr)
}

function run(binary, args, { input, timeoutMs = 90_000 } = {}) {
  return new Promise((done, fail) => {
    const child = spawn(binary, args, { stdio: ["pipe", "pipe", "pipe"] })
    let stdout = "", stderr = ""
    const timer = setTimeout(() => child.kill("SIGTERM"), timeoutMs)
    child.stdout.on("data", chunk => { stdout += chunk })
    child.stderr.on("data", chunk => { stderr += chunk })
    child.on("error", error => { clearTimeout(timer); error.code === "ENOENT" ? fail(new Error(`cannot run ${binary}`)) : done({ code: 1, stdout, stderr: String(error) }) })
    child.on("close", code => { clearTimeout(timer); done({ code: code ?? 1, stdout, stderr }) })
    child.stdin.end(input ?? "")
  })
}

function readPrinted(file) {
  try { return JSON.parse(readFileSync(file, "utf8")) } catch { return [] }
}
function writePrinted(file, ids) {
  const next = `${file}.${process.pid}`
  writeFileSync(next, JSON.stringify(ids.slice(-KEEP_PRINTED)), { mode: 0o600 })
  renameSync(next, file)
}

// Exit 3 of `events wait`: the follow is over. A kernel that is shutting down is an unreachable kernel; a revoked or
// expired grant needs a new `chariox access request`, and the line says what the kernel said.
function followEnded(waited, out) {
  let reason = waited.stderr.trim()
  try { reason = JSON.stringify(JSON.parse(waited.stdout).events.find(event => event.type === "subscription_ended")?.value ?? reason) } catch { /* the plain text stays */ }
  if (/shutting down/i.test(reason)) { out("reviewer kernel unreachable"); return 0 }
  out(`event follow ended: ${reason.slice(0, 300)}`)
  return 0
}

export async function main(argv, out = console.log) {
  const flags = new Map(), sourceNames = []
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i], value = argv[++i]
    if (!["--window-minutes", "--fallback-minutes", "--source", "--chariox", "--state"].includes(flag) || value === undefined) { out(`usage: reviewer-wait-events.mjs [--window-minutes N] [--fallback-minutes N] [--source NAME|ID]... [--chariox PATH] [--state FILE]`); return 1 }
    flag === "--source" ? sourceNames.push(value) : flags.set(flag, value)
  }
  const minutes = (flag, fallback) => { const n = Number(flags.get(flag) ?? fallback); return Number.isFinite(n) && n > 0 ? n : fallback }
  const binary = flags.get("--chariox") ?? "chariox"
  const stateFile = flags.get("--state") ?? join(tmpdir(), "reviewer-wait-events.printed.json")
  const started = Date.now()
  const windowEnd = started + minutes("--window-minutes", 25) * 60_000
  const fallbackMs = minutes("--fallback-minutes", 10) * 60_000
  let fallbackAt = started + fallbackMs
  const printed = readPrinted(stateFile)
  const unreachable = () => { out("reviewer kernel unreachable"); return 0 }
  const chariox = (...args) => run(binary, args)
  const request = body => run(binary, ["request"], { input: JSON.stringify(body) })

  // The sources to follow: the run sources of the workflows, or the ones named with --source.
  const sources = await chariox("events", "sources")
  if (kernelUnreachable(sources)) return unreachable()
  if (sources.code !== 0) { out(`events sources failed: ${sources.stderr.trim().slice(0, 300)}`); return 1 }
  const wanted = JSON.parse(sources.stdout).sources.filter(source => source.kind === "workflow_run" && (sourceNames.length === 0 || sourceNames.some(name => [source.source_id, source.display_name, source.name, source.workflow_id].includes(name))))
  if (wanted.length === 0) { out("no run source to follow: the kernel offers none (workflow missing or this build predates run notifications)"); return 1 }
  // The follow belongs to the grant and outlives this process; follow only what is not followed yet.
  const subscription = await chariox("events", "subscription")
  const followed = subscription.code === 0 ? new Set((JSON.parse(subscription.stdout).subscription?.sources ?? []).map(source => source.source_id)) : new Set()
  for (const source of wanted.filter(source => !followed.has(source.source_id))) {
    const followedNow = await chariox("events", "subscribe", "--source", source.source_id)
    if (kernelUnreachable(followedNow)) return unreachable()
    if (followedNow.code !== 0) { out(`cannot follow ${source.display_name}: ${followedNow.stderr.trim().slice(0, 300)}`); return 1 }
  }
  const sourceIds = new Set(wanted.map(source => source.source_id))

  const lines = []
  const take = ({ at, status, prompt, runId, failure }) => {
    if (printed.includes(runId)) return
    printed.push(runId)
    lines.push(formatLine({ at, status, prompt, runId, failure }))
  }
  // Polling fallback: the workflows' runs that ended since this wait started and were not printed.
  const listRuns = async () => {
    for (const source of wanted) {
      const listed = await request({ ListWorkflowRuns: { session_id: source.session_id, workflow_ref: source.workflow_id, limit: 100 } })
      if (kernelUnreachable(listed)) return false
      if (listed.code !== 0) continue
      for (const entry of JSON.parse(listed.stdout).WorkflowRunsListed?.workflow_runs ?? []) {
        if (!["Completed", "Failed"].includes(entry.status) || (entry.completed_at_ms ?? 0) < started) continue
        take({ at: entry.completed_at_ms, status: entry.status, prompt: entry.invocation_prompt, runId: entry.id, failure: entry.failure_events?.[0]?.message })
      }
    }
    return true
  }
  const finish = () => { writePrinted(stateFile, printed); lines.forEach(line => out(line)); return 0 }

  // The first record skipped as another reader's, in any batch of this wait: acknowledgement is cumulative, so nothing
  // at or after it may be acknowledged, not even by a later batch.
  let cursor, foreignSeen = false
  while (Date.now() < windowEnd) {
    const seconds = Math.max(1, Math.min(MAX_WAIT_SECONDS, Math.ceil((Math.min(windowEnd, fallbackAt) - Date.now()) / 1000)))
    const waited = await chariox("events", "wait", "--timeout", String(seconds), ...(cursor === undefined ? [] : ["--since", String(cursor)]))
    if (kernelUnreachable(waited)) return unreachable()
    let gap = false
    if (waited.code === 3) return followEnded(waited, out)
    if (waited.code === 0) {
      const batch = JSON.parse(waited.stdout)
      // The grant's stream is shared with other sources and targets: only the unbroken run of records this script
      // consumed from the front of the stream is acknowledged, so nobody else's record is dropped with them.
      let acknowledgeThrough
      for (const event of batch.events ?? []) {
        if (event.type === "subscription_ended") return unreachable()
        const mine = event.type === "gap" || (event.type === "notification" && sourceIds.has(event.value.source_id))
        if (!mine) { foreignSeen = true; continue }
        if (event.type === "gap") gap = true
        else {
          const ended = JSON.parse(event.value.output.message)
          take({ at: event.at_ms, status: ended.status, prompt: ended.prompt, runId: ended.run_id, failure: ended.failure })
        }
        if (!foreignSeen) acknowledgeThrough = event.cursor
      }
      // Runs the kernel dropped from the stream come back by listing, whether or not the batch also holds a run.
      if (gap) {
        fallbackAt = Date.now() + fallbackMs
        if (!(await listRuns())) return unreachable()
      }
      if (acknowledgeThrough !== undefined) {
        const acknowledged = await chariox("events", "ack", String(acknowledgeThrough))
        if (kernelUnreachable(acknowledged)) return unreachable()
      }
      cursor = batch.next_cursor
    } else if (waited.code !== 5) { out(`events wait failed: ${waited.stderr.trim().slice(0, 300)}`); return 1 }
    if (lines.length === 0 && Date.now() >= fallbackAt) {
      fallbackAt = Date.now() + fallbackMs
      if (!(await listRuns())) return unreachable()
    }
    if (lines.length > 0) return finish()
  }
  out("no review finished in the window")
  return 0
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then(code => process.exit(code), error => { console.error(String(error.message ?? error)); process.exit(1) })
}
