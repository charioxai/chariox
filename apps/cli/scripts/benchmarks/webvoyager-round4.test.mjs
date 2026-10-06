// MP-08/MP-10/MP-11: vision permission is one protected observation, never Computer mutation.
import test from 'node:test'
import assert from 'node:assert/strict'
import { webVoyagerPrompt, webVoyagerToolAllowed } from './webvoyager-task.mjs'

test('MP-08/MP-10 vision prompt requests protected image blocks and preserves restrictions', () => {
  const prompt = webVoyagerPrompt({ ques: 'Read a plot.', web: 'https://example.org' }, 'recovery-vision-v1')
  assert.match(prompt, /vision-allowed/)
  assert.match(prompt, /slice_screenshot.*return_image_base64=true/)
  assert.match(prompt, /Do not accept optional tracking/)
  assert.match(prompt, /Refresh observed field IDs/)
  assert.match(prompt, /no logins, sign-ups, purchases/)
  assert.match(prompt, /No shell, files, scripts, direct HTTP/)
  assert.match(prompt, /15 mutating Browser actions, 80 Browser tool calls and 600 seconds/)
  assert(!prompt.includes('For image-only results, look for an observed Plain Text'))
})
test('MP-11 allowlist permits only the existing screenshot observation in vision mode', () => {
  for (const variant of ['frozen', 'recovery-v1', 'recovery-vision-v1']) {
    assert(webVoyagerToolAllowed('slice_browser_text', variant))
    assert.equal(webVoyagerToolAllowed('slice_screenshot', variant), variant === 'recovery-vision-v1')
    for (const tool of ['slice_mouse', 'slice_keyboard', 'slice_ocr', 'shell', 'read_file', 'slice_screen_status']) {
      assert.equal(webVoyagerToolAllowed(tool, variant), false)
    }
  }
})

test('MP-10 partial vision campaigns retain invalids and cannot report full eligible rates', async () => {
  const { compareRound4 } = await import('./webvoyager-round4-report.mjs')
  const good = { harnessValid: true, judgeValid: true, judgeVerdict: 'SUCCESS' }
  const old = [{ taskId: 'A--0', ...good }, { taskId: 'B--0', ...good, judgeVerdict: 'NOT SUCCESS', answer: 'cookie consent' }, { taskId: 'B--1', harnessValid: false }]
  const now = [{ taskId: 'A--0', harnessValid: true, judgeValid: false }, { taskId: 'B--0', ...good }]
  const partial = compareRound4(old, now, 3)
  assert.deepEqual(partial.round4VisionAllowed, { tasks: 2, wins: 1, valid: 1, invalid: 1, rate: 0.5, eligible: 3, unattempted: 1, eligibleRate: null })
  assert.equal(partial.paired.gained, 1); assert.equal(partial.paired.lost, 1)
  assert.equal(partial.byBaselineClass.find(c => c.category.includes('consent')).afterWins, 1)
  assert.equal(partial.perSite.find(s => s.site === 'B').unattempted, 1)
  assert.equal(partial.top3RemainingLossClasses[0].category, 'harness/runtime error')
  assert.equal(compareRound4(old, [...now, { taskId: 'B--1', ...good }], 3).round4VisionAllowed.eligibleRate, 2 / 3)
  assert.throws(() => compareRound4(old, [now[0], now[0]], 3))
})


test('MP-10 over-budget vision calls remain invalid without inventing a cleanup failure', async () => {
  const { compareRound4 } = await import('./webvoyager-round4-report.mjs')
  const baseline = [{ taskId: 'S--0', harnessValid: true, judgeValid: true, judgeVerdict: 'SUCCESS' }]
  const row = { ...baseline[0], harnessValid: false, providerToolCalls: 82, maxToolCalls: 80, cleanupValid: true }
  const report = compareRound4(baseline, [row], 1)
  assert.equal(report.round4VisionAllowed.wins, 0); assert.equal(report.round4VisionAllowed.invalid, 1)
  assert.equal(report.top3RemainingLossClasses[0].category, 'timeout / step budget')
  assert.deepEqual(report.budgetViolations, [{ taskId: 'S--0', providerToolCalls: 82, maxToolCalls: 80 }])
  assert.equal(row.cleanupValid, true); assert.equal(row.judgeVerdict, 'SUCCESS')
})
