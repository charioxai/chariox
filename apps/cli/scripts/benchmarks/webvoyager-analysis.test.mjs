// MP-08/MP-10: diagnostics never rescore immutable baseline rows.
import test from 'node:test'
import assert from 'node:assert/strict'
import { classifyFailure, stratifiedSample } from './webvoyager-analysis.mjs'
import { webVoyagerPrompt } from './webvoyager-task.mjs'
import { captureFinalEvidence } from './webvoyager-final-capture.mjs'
const loss = answer => ({ taskId: 'site--1', harnessValid: true, judgeValid: true, judgeVerdict: 'NOT SUCCESS', answer })
test('primary failure taxonomy distinguishes consent, challenge, expired dates, and extraction', () => {
  assert.equal(classifyFailure(loss('Cookie consent blocked navigation')).subclass, 'consent_policy')
  assert.equal(classifyFailure(loss('Cloudflare verification blocked the site')).category, 'site blocked / anti-bot / captcha')
  assert.equal(classifyFailure(loss('January 2024 is in the past; cookie consent also appeared')).category, 'live-site drift (answer changed)')
  assert.equal(classifyFailure(loss('Result headings but no readable numerical values')).category, 'reading/extraction error')
  assert.equal(classifyFailure({ ...loss(''), harnessValid: false, firstFailingSeam: 'final_capture' }).category, 'harness/runtime error')
  assert.equal(classifyFailure({ ...loss(''), harnessValid: false, budgetExceeded: 'wall_time' }).category, 'timeout / step budget')
  assert.equal(classifyFailure({ ...loss(''), judgeVerdict: 'SUCCESS' }), null)
})
test('stratified sample is deterministic, includes controls, and never duplicates or includes exclusions', () => {
  const rows = Array.from({ length: 100 }, (_, i) => ({ ...loss(i < 30 ? 'cookie consent' : 'unreadable result'), taskId: `site--${i}`, judgeVerdict: i < 20 ? 'SUCCESS' : 'NOT SUCCESS' }))
  rows.push({ taskId: 'excluded', excluded: true })
  const a = stratifiedSample(rows, 60)
  assert.equal(a.length, 60); assert.equal(new Set(a.map(x => x.taskId)).size, 60)
  assert(a.some(x => x.stratum === 'win')); assert(!a.some(x => x.taskId === 'excluded'))
  assert.deepEqual(a, stratifiedSample([...rows].reverse(), 60))
  assert.throws(() => stratifiedSample(rows, 101))
})
test('recovery prompt authorizes only privacy-preserving UI choices and preserves task/budgets', () => {
  const task = { ques: 'Find the cheapest item.', web: 'https://example.org' }
  const old = webVoyagerPrompt(task), updated = webVoyagerPrompt(task, 'recovery-v1')
  assert.notEqual(old, updated); assert(updated.includes(task.ques)); assert(updated.includes(task.web))
  assert.match(updated, /Reject all|I do not agree/); assert.match(updated, /Do not accept optional tracking/)
  assert.match(updated, /15 mutating Browser actions, 80 Browser tool calls and 600 seconds/)
  assert.match(updated, /No shell, files, scripts, direct HTTP/); assert.match(updated, /no logins, sign-ups, purchases/)
  assert.throws(() => webVoyagerPrompt(task, 'typo'))
})
test('final screenshot retry retains original error, requires fresh capture, and never retries indefinitely', async () => {
  let calls = 0
  const errors = []
  await captureFinalEvidence({ capture: async () => { calls++; if (calls === 1) { errors.push('timeout'); throw Error('capture') } }, pause: async () => {} })
  assert.equal(calls, 2); assert.deepEqual(errors, ['timeout'])
  calls = 0
  await assert.rejects(captureFinalEvidence({ capture: async () => { calls++; throw Error('capture') }, pause: async () => {} }))
  assert.equal(calls, 3)
})
test('MP-11 runtime rejects cross-lane and traversal paths before creating state', async () => {
  const { startOwnedRuntime } = await import('./round2/runtime.mjs')
  const base = { lane: '/root/.chariox/dev/browser-resume-20260930/agents/wvanalysis/sample',
    evidence: '/root/.codex/evidence/browser-resume-20260930/wvanalysis/sample', release: '/tmp/missing-release',
    repo: '/root/work/agent-wvanalysis', clientRoot: '/tmp/missing-client', accountHome: '/tmp/missing-profile',
    image: `sha256:${'a'.repeat(64)}`, label: 'wvanalysis-sample', laneName: 'wvanalysis', sandboxCompatibility: false }
  for (const change of [{ laneName: 'other' }, { lane: '/root/.chariox/dev/browser-resume-20260930/agents/r2next/sample' },
    { evidence: '/root/.codex/evidence/browser-resume-20260930/wvanalysis/../r2next/sample' }, { label: 'r2next-sample' }]) {
    await assert.rejects(startOwnedRuntime({ ...base, ...change }))
  }
})
