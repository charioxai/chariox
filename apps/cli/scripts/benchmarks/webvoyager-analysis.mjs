// MP-08/MP-10: primary observed seam, with conservative evidence labels; never a scoring rule.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
export const categories = [
  'site blocked / anti-bot / captcha', 'live-site drift (answer changed)', 'judge disagreement',
  'navigation failure', 'reading/extraction error', 'timeout / step budget', 'harness/runtime error', 'other',
]
export function classifyFailure(row) {
  if (row.excluded || (row.harnessValid && row.judgeValid && row.judgeVerdict === 'SUCCESS')) return null
  const answer = row.answer ?? ''
  const result = (category, subclass, match = null) => ({ category, subclass, basis: match?.[0] ?? row.firstFailingSeam ?? 'answer/judge review', confidence: 'reported seam; class sample reviewed' })
  const rule = (regex, category, subclass) => { const match = answer.match(regex); return match && result(category, subclass, match) }
  if (row.budgetExceeded) return result('timeout / step budget', row.budgetExceeded)
  if (/within the 15-action limit/i.test(answer)) return result('timeout / step budget', 'self_reported_step_budget', answer.match(/within the 15-action limit/i))
  if (!row.harnessValid || !row.judgeValid) return result('harness/runtime error', row.firstFailingSeam ?? 'cleanup')
  return rule(/Cloudflare|CloudFront|\b403\b|captcha|robots|access den(?:ied|ial)|request blocked|human verification|Vercel Security Checkpoint/i, categories[0], 'anti_bot')
    ?? rule(/expired dates?|dates?.{0,80}(?:in the past|expired)|(?:January|February|March).{0,100}in the past|past dates/i, categories[1], 'expired_date')
    ?? rule(/cookie|consent/i, categories[3], 'consent_policy')
    ?? rule(/SKIPPED_STATE_CHANGE/i, categories[7], 'forbidden_task_action')
    ?? rule(/(?:requires?|required|needs?|blocked.{0,30}) (?:a |an )?(?:log.?in|sign.?in|account)|sign.in to see|login.{0,50}(?:required|prohibited)/i, categories[0], 'login_required')
    ?? rule(/(?:404|can't find the page|no M3(?: Max)? option|no 64GB option|no AirPods \(3rd generation\) listing|no longer (?:available|listed)|discontinued|Available at authorized resellers)/i, categories[1], 'removed_data')
    ?? rule(/USD|euros?|EUR prices|cannot ship|delivery restrictions/i, categories[3], 'locale_delivery')
    ?? rule(/Frame with the given frameId is not found|non.actionable|browser_element_|stale.element|focus errors|obscured|disabled|navigation.{0,40}fail|sort.{0,40}(?:inaccessible|failed)/i, categories[3], 'element_navigation')
    ?? rule(/readable|unreadable|rendered as an image|text or captions|embedded.{0,20}pictures|no numerical|could(?:n.t| not) (?:verify|confirm|locate)|missing (?:data|.*price)|unverified|closest match|no results|could not establish|does not (?:document|establish|specify)|no .*filter|not 2022/i, categories[4], 'missing_or_partial_extraction')
    ?? result(categories[4], 'answer_incomplete_or_incorrect')
}
export function stratum(row) {
  const failure = classifyFailure(row)
  return failure ? `${failure.category} / ${failure.subclass}` : 'win'
}
export function stratifiedSample(rows, size = 60) {
  const eligible = rows.filter(row => !row.excluded)
  assert(Number.isSafeInteger(size) && size > 0 && size <= eligible.length, 'MP-10 invalid sample size')
  assert.equal(new Set(eligible.map(row => row.taskId)).size, eligible.length, 'MP-10 duplicate baseline task')
  const groups = new Map()
  for (const row of eligible) {
    const key = stratum(row)
    if (!groups.has(key)) groups.set(key, [])
    groups.get(key).push(row)
  }
  // Largest remainder allocation proportional to prevalence, at least one per nonempty stratum.
  assert(size >= groups.size, 'MP-10 sample cannot cover every nonempty stratum')
  const quotas = [...groups].map(([key, values]) => ({ key, values, n: 1, target: size * values.length / eligible.length }))
  for (let allocated = quotas.length; allocated < size; allocated++) {
    const next = quotas.filter(g => g.n < g.values.length).sort((a, b) => (b.target - b.n) - (a.target - a.n) || a.key.localeCompare(b.key))[0]
    next.n++
  }
  const hash = id => createHash('sha256').update(`MP-08/MP-10-wvanalysis-v1:${id}`).digest('hex')
  return quotas.flatMap(g => g.values.sort((a, b) => hash(a.taskId).localeCompare(hash(b.taskId))).slice(0, g.n)
    .map(row => ({ taskId: row.taskId, stratum: g.key, population: g.values.length, selectedInStratum: g.n })))
    .sort((a, b) => hash(a.taskId).localeCompare(hash(b.taskId)))
}
