import { messagesSince } from './perf-budget-tighten-only.mjs'

// No agent tool is exposed unless needed. The tools a provider's agent holds before its first call are ONE list,
// apps/kernel/agent_upfront_tools.json: {provider: {role: [tool names] | "*"}} ("*" = the whole catalog, only ever shrinks).
// The kernel reads that file (tool_loading.rs); nothing else may name a load-upfront tool or the Claude `alwaysLoad` key.
// Diff-aware: a name added to a list, or "*" where the base had a list, is red unless a commit since the merge base carries
// `budget raised: agent_upfront_tools.<provider>.<role>: <why>`. Upfront bytes per provider are ceilings in
// apps/kernel/agent_context_budget.json, held by perf-budget-tighten-only. Cloud has no agent runtime tools: not applicable.
const LIST = 'apps/kernel/agent_upfront_tools.json'
const LOADER = 'apps/kernel/src/transport/mcp_server/tool_loading.rs'
const RAISED = /^budget raised: (agent_upfront_tools\.[\w.-]+?):? /gm

function lists(text) {
  const out = new Map()
  const problems = []
  let parsed
  try { parsed = JSON.parse(text) } catch (error) { return { out, problems: [`is not JSON (${error.message})`] } }
  for (const [provider, roles] of Object.entries(parsed ?? {})) {
    for (const [role, names] of Object.entries(roles ?? {})) {
      const id = `agent_upfront_tools.${provider}.${role}`
      if (names === '*') out.set(id, '*')
      else if (Array.isArray(names) && names.every(n => typeof n === 'string') && new Set(names).size === names.length) out.set(id, names)
      else problems.push(`${id} is neither "*" nor a list of distinct tool names`)
    }
  }
  return { out, problems }
}

export default {
  name: 'agent-tools-on-demand',
  ratchet: 'diff',
  rule: 'no agent tool is exposed unless needed: one upfront list per provider and role, growth needs a "budget raised:" line approved in review',
  rootFix: 'leave the tool deferred (found through tool search / chariox.tools.find), or add "budget raised: agent_upfront_tools.<provider>.<role>: <why>" to a commit message and get it approved in review',
  scan(ctx) {
    const findings = []
    const text = ctx.read(LIST)
    if (text === null) return [{ file: LIST, line: 0, what: 'the one list of upfront agent tools is missing' }]
    const now = lists(text)
    findings.push(...now.problems.map(what => ({ file: LIST, line: 0, what })))
    const loader = ctx.read(LOADER) ?? ''
    if (!loader.includes('agent_upfront_tools.json')) findings.push({ file: LOADER, line: 0, what: `does not read ${LIST}: the upfront list must be in one place` })
    for (const hit of ctx.grep('anthropic/alwaysLoad', { paths: ['apps', 'packages', 'adapters'] })) {
      if (hit.file === LOADER || /(^|\/)tests?(\/|\.rs$)|_tests?\.rs$/.test(hit.file)) continue
      findings.push({ file: hit.file, line: hit.line, what: 'marks a tool load-upfront outside tool_loading.rs' })
    }
    if (!ctx.base) return findings
    const before = ctx.readAtBase(LIST)
    if (before === null) return findings
    const atBase = lists(before).out
    const grown = []
    for (const [id, names] of now.out) {
      const was = atBase.get(id)
      if (was === undefined) { grown.push(`${id} is a new upfront list`); continue }
      if (was === '*') continue
      if (names === '*') grown.push(`${id} is raised from a list to "*" (every tool)`)
      else for (const name of names) if (!was.includes(name)) grown.push(`${id} gains ${name}`)
    }
    if (!grown.length) return findings
    const history = messagesSince(ctx.root, ctx.base)
    if (history.error) return [...findings, { file: LIST, line: 0, what: history.error }]
    const raised = new Set([...history.messages.matchAll(RAISED)].map(match => match[1]))
    for (const what of grown) {
      const id = /^(agent_upfront_tools\.[\w-]+\.[\w-]+)/.exec(what)[1]
      if (!raised.has(id)) findings.push({ file: LIST, line: 0, what: `${what} without a "budget raised: ${id}: <why>" line in a commit since the merge base` })
    }
    return findings
  },
}
