import { gitOut } from '../lib/ratchet.mjs'

// Performance budgets only tighten (docs/PERFORMANCE_BUDGETS.md). A budget file holds ceilings: perf-budgets/<side>.json maps a
// budget name to {max, unit, gate, measured}, and apps/kernel/agent_context_budget.json maps a name to its measured size.
// Diff-aware: a ceiling that is higher than at the merge base, or gone, is red unless a commit since the base carries the line
// `budget raised: <side>.<name>: <why>` for it, which review approves. A lower ceiling or a new budget always passes.
const BUDGET_FILE = /^perf-budgets\/[a-z0-9-]+\.json$/
const AGENT_FILE = 'apps/kernel/agent_context_budget.json'
const RAISED = /^budget raised: ([\w.-]+?):? /gm

export default {
  name: 'perf-budget-tighten-only',
  ratchet: 'diff',
  rule: 'docs/PERFORMANCE_BUDGETS.md: a performance budget only tightens; a raise needs a "budget raised:" line approved in review',
  rootFix: 'bring the measured number back under the budget, or add the line "budget raised: <id>: <why>" to the commit message and get it approved in review',
  scan(ctx) {
    const findings = []
    const files = ctx.files.filter(file => BUDGET_FILE.test(file) || file === AGENT_FILE)
    const ceilings = new Map()
    for (const file of files) {
      const read = budgets(file, ctx.read(file))
      findings.push(...read.problems.map(what => ({ file, line: 0, what })))
      ceilings.set(file, read.ceilings)
    }
    if (!ctx.base) return findings
    const atBase = gitOut(ctx.root, 'ls-tree', '-r', '--name-only', ctx.base).split('\n').filter(file => BUDGET_FILE.test(file) || file === AGENT_FILE)
    const raised = new Set([...gitOut(ctx.root, 'log', '--format=%B', `${ctx.base}..HEAD`).matchAll(RAISED)].map(match => match[1]))
    for (const file of atBase) {
      const now = ceilings.get(file) ?? new Map()
      for (const [name, before] of budgets(file, ctx.readAtBase(file)).ceilings) {
        const id = `${budgetSide(file)}.${name}`
        const after = now.get(name)
        if (after !== undefined && after <= before) continue
        if (raised.has(id)) continue
        const what = after === undefined ? `budget ${id} (${before}) is removed` : `budget ${id} is raised from ${before} to ${after}`
        findings.push({ file, line: lineOf(ctx.read(file) ?? '', name), what: `${what} without a "budget raised: ${id}: <why>" line in a commit since the merge base` })
      }
    }
    return findings
  },
}

const budgetSide = file => file === AGENT_FILE ? 'agent_context_budget' : file.slice('perf-budgets/'.length, -'.json'.length)

const lineOf = (text, name) => text.split('\n').findIndex(line => line.includes(`"${name}"`)) + 1

// {ceilings: Map name -> number, problems: [what]} for one budget file's text; a file that is not in the format is a problem.
function budgets(file, text) {
  const ceilings = new Map()
  const problems = []
  if (text === null) return { ceilings, problems }
  let parsed
  try { parsed = JSON.parse(text) } catch (error) { return { ceilings, problems: [`is not JSON (${error.message})`] } }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { ceilings, problems: ['is not an object of budgets'] }
  for (const [name, budget] of Object.entries(parsed)) {
    if (file === AGENT_FILE) {
      if (Number.isFinite(budget)) ceilings.set(name, budget)
      else problems.push(`${name} is not a number`)
      continue
    }
    const missing = ['unit', 'gate', 'measured'].filter(key => typeof budget?.[key] !== 'string' || !budget[key].trim())
    if (!Number.isFinite(budget?.max)) missing.unshift('max')
    if (missing.length) problems.push(`budget ${name} lacks ${missing.join(', ')} (each budget is {max, unit, gate, measured})`)
    else ceilings.set(name, budget.max)
  }
  return { ceilings, problems }
}
