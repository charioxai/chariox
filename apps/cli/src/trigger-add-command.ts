import { tokenizeShellLine } from "@chariox/kernel-client/shell-core"
import type { AgentHttpTriggerOptions, AgentTriggerSource } from "@chariox/kernel-client/kernel-types"

type ScheduleSource = Omit<Extract<AgentTriggerSource, { kind: "schedule" }>, "next_run_at_ms">
export type AddTriggerAction = {
  type: "add"
  source: ScheduleSource | ({ kind: "http" } & Partial<AgentHttpTriggerOptions>)
  busyPolicy: "queue" | "steer"
  prompt?: string
}

export const TRIGGER_ADD_USAGE = "usage: /trigger add schedule (--every <N>s|m|h | --cron '<sec min hour day month weekday>' [--timezone <IANA zone>]) [--prompt <text>] [--queue|--steer] | /trigger add http [--route <path>] [--methods GET|POST|GET,POST] [--parser json|query_params|webhook|path_template] [--template <path>] [--mode async|sync] [--sync-timeout-ms <ms>] [--poll-ms <ms>] [--input-schema '<json>'] [--prompt <text>] [--queue|--steer]"

/** Keep schedule flags aligned with the room's `trigger add schedule` command. The kernel validates source options. */
export function parseTriggerAdd(command: string): AddTriggerAction {
  const [, , kind, ...words] = tokenizeShellLine(command)
  if (kind !== "schedule" && kind !== "http") throw new Error(TRIGGER_ADD_USAGE)
  const allowed = new Set(kind === "schedule" ? ["--every", "--cron", "--timezone", "--prompt"]
    : ["--route", "--methods", "--parser", "--template", "--mode", "--sync-timeout-ms", "--poll-ms", "--input-schema", "--prompt"])
  const options = new Map<string, string>()
  let busyPolicy: "queue" | "steer" | undefined
  for (let index = 0; index < words.length; index++) {
    const flag = words[index]!
    if (flag === "--queue" || flag === "--steer") {
      if (busyPolicy) throw new Error("give only one of --queue or --steer")
      busyPolicy = flag === "--steer" ? "steer" : "queue"
      continue
    }
    if (!allowed.has(flag)) throw new Error(`unknown option ${flag}; ${TRIGGER_ADD_USAGE}`)
    if (options.has(flag)) throw new Error(`${flag} is given twice`)
    const value = words[++index]
    if (!value || value.startsWith("--")) throw new Error(`${flag} needs a value`)
    options.set(flag, value)
  }
  const prompt = options.get("--prompt")?.trim()
  return { type: "add", source: kind === "schedule" ? scheduleSource(options) : httpSource(options), busyPolicy: busyPolicy ?? "queue", ...(prompt ? { prompt } : {}) }
}

function scheduleSource(options: Map<string, string>): ScheduleSource {
  const every = options.get("--every"), cron = options.get("--cron"), timezone = options.get("--timezone")
  if ((!every && !cron) || (every && cron) || (every && timezone)) throw new Error(`give either --every or --cron; ${TRIGGER_ADD_USAGE}`)
  let trigger: ScheduleSource["trigger"]
  if (every) {
    const match = /^(\d+)(s|m|h)$/.exec(every)
    const seconds = match ? Number(match[1]) * ({ s: 1, m: 60, h: 3600 }[match[2]!] ?? 0) : 0
    if (!Number.isSafeInteger(seconds) || seconds <= 0) throw new Error("--every must be a positive number of seconds, minutes or hours, such as 30m")
    trigger = { kind: "interval", every_seconds: seconds }
  } else {
    trigger = { kind: "cron", expression: cron!, timezone: timezone ?? "UTC" }
  }
  return { kind: "schedule", trigger, repeat: "every", overlap_policy: "skip", max_runs: null }
}

function httpSource(options: Map<string, string>): AddTriggerAction["source"] {
  const source: { kind: "http" } & Partial<AgentHttpTriggerOptions> = { kind: "http" }
  const route = options.get("--route"), methods = options.get("--methods"), parser = options.get("--parser"), template = options.get("--template"), mode = options.get("--mode")
  if (route) source.route = route
  if (methods) {
    source.methods = methods.split(",").map(method => {
      if (method !== "GET" && method !== "POST") throw new Error("--methods must be GET, POST or GET,POST")
      return method
    })
  }
  if (template && parser !== "path_template") throw new Error("--template requires --parser path_template")
  if (parser === "path_template") {
    if (!template) throw new Error("--parser path_template requires --template")
    source.parser = { kind: "path_template", template }
  } else if (parser) {
    if (parser !== "json" && parser !== "query_params" && parser !== "webhook") throw new Error("--parser must be json, query_params, webhook or path_template")
    source.parser = { kind: parser }
  }
  if (mode) {
    if (mode !== "sync" && mode !== "async") throw new Error("--mode must be async or sync")
    source.mode = mode
  }
  for (const [flag, field] of [["--sync-timeout-ms", "sync_timeout_ms"], ["--poll-ms", "poll_ms"]] as const) {
    const value = options.get(flag)
    if (!value) continue
    const number = Number(value)
    if (!Number.isSafeInteger(number) || number <= 0) throw new Error(`${flag} must be a positive integer`)
    source[field] = number
  }
  const schema = options.get("--input-schema")
  if (schema) {
    try { source.input_schema = JSON.parse(schema) } catch { throw new Error("--input-schema must be valid JSON") }
  }
  return source
}
