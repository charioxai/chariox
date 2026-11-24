// Turns a session's history rows (SessionHistoryEntry: kind, text, timestamp_ms, agent_id) into chariox-loadlab-trace v1 files that keep only
// the kinds, sizes and timings of the session. Nothing a person or a provider wrote leaves the converter: no text, no tool input or output,
// no ids, aliases, authors, attachment names or paths. A value the Vault filled into a prompt, a tool call or a terminal is text, and text is
// counted, never copied. Every output line is built from numbers and a closed vocabulary, then checked by the same parser the replay uses.
const { parseTrace } = require('../../../apps/kernel/src/provider/registry/dev_stub_adapter/trace_format.cjs')

const MAX_MS = 2_147_483_647
const MAX_BYTES = 4 * 1024 * 1024
const MAX_STREAM_MS = 10 * 60 * 1000
const clampBytes = value => Math.min(MAX_BYTES, Math.max(1, Math.round(value)))
const clampMs = value => Math.min(MAX_MS, Math.max(0, Math.round(value)))

/** One of four labels, from the shape of the text only. */
function markdownOf(text) {
  if (text.includes('```')) return 'code'
  const lines = text.split('\n').filter(line => line.trim())
  if (lines.some(line => /^\s*\|.*\|\s*$/.test(line)) && lines.some(line => /^\s*\|?\s*:?-{3,}/.test(line))) return 'table'
  if (lines.filter(line => /^\s*([-*]|\d+[.)])\s/.test(line)).length >= 2) return 'list'
  return 'plain'
}

/**
 * A tool name is kept only when it is a name a provider gives its own tools (Bash, Read, mcp__x__y, chariox.y): the first word of the row,
 * and only from the closed shapes below. Anything else becomes "Tool", so free text can never ride in the name field.
 */
function toolName(text) {
  const first = String(text).trimStart().split(/[\s:(]/, 1)[0]
  return /^(?:[A-Z][A-Za-z]{2,24}|mcp__[a-z0-9_]{1,60}|chariox\.[a-z_.]{1,40})$/.test(first) ? first : 'Tool'
}

/**
 * What the converter keeps of a row, and nothing else. A row from the kernel's history outline carries total_chars (the size of the whole
 * entry) beside a fragment of its text; the size is read from there, and the shape (markdown label, tool name) from the fragment.
 */
function keep(row) {
  const text = typeof row.text === 'string' ? row.text : ''
  return {
    kind: row.kind, agent_id: row.agent_id ?? '', timestamp_ms: row.timestamp_ms, steering: row.steering === true,
    size: clampBytes(Number.isSafeInteger(row.total_chars) ? row.total_chars : text.length),
    markdown: markdownOf(text), tool: toolName(text),
  }
}

/**
 * The history outline of GetSessionHistoryOutline (agents -> turns -> user_prompt, entries, summary, blobs) as flat rows.
 * A turn has one prompt, its own `user_prompt`; any other prompt in its entries or blobs is a steering prompt that belongs to the turn,
 * so it is marked and never opens a turn of its own. When a final reply is long, the kernel keeps a truncated `summary` and forces blobs
 * over the same events: the summary is dropped when a blob of its own kind covers its sequence, so the reply is recorded once; a blob of another kind whose range spans it does not hold it.
 */
function rowsFromOutline(outline) {
  const row = (agent, kind, timestamp_ms, total_chars, text, steering) => ({ kind, agent_id: agent.agent_id, timestamp_ms, total_chars, text, steering })
  return outline.agents.flatMap(agent => agent.turns.flatMap(turn => {
    const covered = page => Number.isSafeInteger(page.entry_index) && turn.blobs.some(blob => blob.kind === page.entry.kind && blob.sequence_start <= page.entry_index && page.entry_index <= blob.sequence_end)
    const page = ({ total_chars, entry }, steering) => row(agent, entry.kind, entry.timestamp_ms, total_chars, entry.text, steering)
    return [
      page(turn.user_prompt, false),
      ...turn.entries.map(entry => page(entry, entry.entry.kind === 'user_prompt')),
      ...(turn.summary && !covered(turn.summary) ? [page(turn.summary, false)] : []),
      // A blob stands for a run of collapsed entries: its kind, size and time only (its title, summary and preview are text and are never read).
      ...turn.blobs.map(blob => row(agent, blob.kind, blob.timestamp_ms, blob.total_chars, undefined, blob.kind === 'user_prompt')),
    ]
  }))
}

function turnsOf(entries) {
  const turns = []
  let turn = null
  for (const entry of entries) {
    if (entry.steering) continue
    if (entry.kind === 'user_prompt') {
      turn = { start: entry.timestamp_ms, rows: [entry], error: false }
      turns.push(turn)
    } else if (turn && ['provider_output', 'provider_reasoning', 'provider_tool', 'provider_error'].includes(entry.kind)) {
      turn.rows.push(entry)
      if (entry.kind === 'provider_error') turn.error = true
    }
  }
  return turns
}

function traceFor(index, entries) {
  const lines = [{ v: 1, source: 'recorded', seed: 0, agent: `agent-${index + 1}`, notes: 'kinds, sizes and timings of a recorded session' }]
  const turns = turnsOf(entries)
  if (turns.length === 0) return null
  for (const turn of turns) {
    let last = 0
    const at = row => { last = Math.max(last, clampMs(row.timestamp_ms - turn.start)); return last }
    for (const [position, row] of turn.rows.entries()) {
      const next = turn.rows[position + 1]
      const span = next ? Math.min(MAX_STREAM_MS, Math.max(0, next.timestamp_ms - row.timestamp_ms)) : 0
      const size = row.size
      if (row.kind === 'user_prompt') lines.push({ t: 0, kind: 'prompt', chars: size })
      else if (row.kind === 'provider_reasoning') lines.push({ t: at(row), kind: 'reasoning', chars: size, stream_ms: clampMs(span) })
      else if (row.kind === 'provider_output') lines.push({ t: at(row), kind: 'text', chars: size, stream_ms: clampMs(span), markdown: row.markdown })
      else if (row.kind === 'provider_tool') {
        lines.push({ t: at(row), kind: 'tool', name: row.tool, input_bytes: clampBytes(size / 10), output_bytes: size, ms: clampMs(span), outcome: 'ok' })
      }
    }
    lines.push({ t: last, kind: 'end', outcome: turn.error ? 'error' : 'completed' })
  }
  return `${lines.map(line => JSON.stringify(line)).join('\n')}\n`
}

/** entries: history rows of one session. Returns a Map of `agent-N.jsonl` to its trace; agents are numbered in order of first appearance. */
function convertHistory(rows) {
  const byAgent = new Map()
  for (const entry of rows.map(keep)) {
    const key = entry.agent_id ?? ''
    if (!byAgent.has(key)) byAgent.set(key, [])
    byAgent.get(key).push(entry)
  }
  const traces = new Map()
  let index = 0
  for (const rows of byAgent.values()) {
    const sorted = rows.slice().sort((a, b) => a.timestamp_ms - b.timestamp_ms)
    const contents = traceFor(index, sorted)
    if (contents === null) continue
    const filename = `agent-${index + 1}.jsonl`
    parseTrace(contents, filename)
    traces.set(filename, contents)
    index++
  }
  return traces
}

module.exports = { convertHistory, markdownOf, rowsFromOutline, toolName }

if (require.main === module) {
  const { mkdirSync, readFileSync, writeFileSync } = require('node:fs')
  const { join } = require('node:path')
  const [input, output] = process.argv.slice(2)
  if (!input || !output) throw new Error('usage: node scripts/drills/loadlab/history-to-trace.cjs <history-rows.json> <output-directory>')
  const rows = JSON.parse(readFileSync(input, 'utf8'))
  const traces = convertHistory(Array.isArray(rows) ? rows : rows.agents ? rowsFromOutline(rows) : rows.entries)
  mkdirSync(output, { recursive: true, mode: 0o700 })
  for (const [filename, contents] of traces) writeFileSync(join(output, filename), contents, { flag: 'wx', mode: 0o600 })
  console.log(`${traces.size} trace file(s) in ${output}`)
}
