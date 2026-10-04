// MP-08 / MP-10 / MP-11 H3/H4. Owner policy, independent of site or gold.
const formats = {
  text: 'the exact final answer text required by the task',
  name: 'one JSON string containing only the requested name',
  set: 'one JSON array of unique strings containing only the requested items',
  number: 'one JSON number containing only the requested numeric value',
  records: 'one JSON array of objects containing only the declared fields',
}

function answerContract(contract) {
  if (!contract || !Object.hasOwn(formats, contract.kind)
    || Object.keys(contract).some(key => !['kind', 'fields', 'taskId'].includes(key))
    || (Object.hasOwn(contract, 'taskId') && !(typeof contract.taskId === 'string' && contract.taskId.length
      || Number.isSafeInteger(contract.taskId)))
    || (contract.kind === 'records' ? !Array.isArray(contract.fields) || !contract.fields.length
      || contract.fields.some(field => typeof field !== 'string' || !field.length)
      || new Set(contract.fields).size !== contract.fields.length : Object.hasOwn(contract, 'fields'))) {
    throw new Error('MP-08 / MP-10 / MP-11 invalid declared answer contract')
  }
  return Object.freeze({ ...contract, ...(contract.fields ? { fields: Object.freeze([...contract.fields]) } : {}) })
}

// The adapter must classify an observed control's meaning; never guess it from
// a label or turn this policy into a site-specific click/selector shortcut.
export function consentDecision(action) {
  const allowed = action === 'reject_nonessential'
  return { allowed, reason: allowed ? 'reject_nonessential' : 'nonessential_consent_forbidden' }
}

export function prepareRound2Prompt({ prompt, startedAt, questionMode = 'live', answerContract: declared = { kind: 'text' } }) {
  if (typeof prompt !== 'string' || !prompt.trim()) throw new Error('MP-10 public task prompt required')
  if (typeof startedAt !== 'string' || !/T.*(?:Z|[+-]\d\d:\d\d)$/.test(startedAt)
    || !Number.isFinite(Date.parse(startedAt))) throw new Error('MP-10 valid admission date with timezone required')
  if (!['live', 'fixed'].includes(questionMode)) throw new Error('MP-10 declared question mode required')
  const contract = answerContract(declared), runDate = new Date(startedAt).toISOString().slice(0, 10)
  const policy = Object.freeze({ consent: 'reject_nonessential', runDate, timezone: 'UTC', questionMode,
    answerContract: contract, scoring: 'official_verdict_unchanged' })
  const instructions = [
    'MP-08 / MP-10 / MP-11 round-2 owner policy:',
    'Always reject non-essential consent through an observed ordinary Browser UI control. Choose reject-all or necessary-only only when it clearly rejects every optional category. Never accept optional consent, blanket click-through, authorize access, or bypass CAPTCHA. If rejection is unavailable or ambiguous, skip that source and record the blocker. This limited consent permission does not authorize other mutations.',
    `Run date: ${runDate} (UTC). ${questionMode === 'live' ? 'Answer live questions as of this run date using independently checked accessible sources. Use an explicit historical date in the task when one is specified.' : 'Use the date specified in the task; the run date is provenance only.'} Never substitute historical gold for observed facts. Keep evaluator data and feedback outside your context. Historical-gold disagreements retain the official verdict.`,
  ]
  if (contract.kind !== 'text' || Object.hasOwn(contract, 'taskId')) {
    instructions.push(`Final answer format: ${formats[contract.kind]}.${contract.fields ? ` Declared fields: ${JSON.stringify(contract.fields)}.` : ''}`)
    if (Object.hasOwn(contract, 'taskId')) instructions.push(`Return ONLY one JSON object with exactly two keys: "id" equal to ${JSON.stringify(contract.taskId)} and "answer" containing that value.`)
    instructions.push('No prose, citations, Markdown fences, or extra fields in the final answer unless requested as answer content. Do not change facts to fit the format. If unsupported, use an empty string for a name/number/text answer or an empty array for a set/records answer.')
  }
  // Policy follows public instructions so the narrow consent permission is
  // explicit even when an adapter's task prompt says read-only browsing.
  return { prompt: `${prompt}\n\n${instructions.join('\n')}`, policy }
}

export function packageRound2Answer(text, declared = { kind: 'text' }) {
  const contract = answerContract(declared)
  const invalid = () => { throw new Error('MP-08 / MP-10 / MP-11 final answer shape violates declared contract') }
  if (typeof text !== 'string') invalid()
  if (contract.kind === 'text' && !Object.hasOwn(contract, 'taskId')) return text
  let value
  try { value = JSON.parse(text) } catch { invalid() }
  if (Object.hasOwn(contract, 'taskId')) {
    if (!value || Array.isArray(value) || typeof value !== 'object' || value.id !== contract.taskId
      || Object.keys(value).length !== 2 || !Object.hasOwn(value, 'answer')) invalid()
    value = value.answer
  }
  switch (contract.kind) {
    case 'text': case 'name':
      if (typeof value !== 'string') invalid()
      return value
    case 'set':
      if (!Array.isArray(value) || value.some(item => typeof item !== 'string' || !item.trim())
        || new Set(value).size !== value.length) invalid()
      return value
    case 'number':
      if (value === '') return '' // Explicit unsupported answer, never a guessed zero.
      if (typeof value !== 'number' || !Number.isFinite(value)
        || Number.isInteger(value) && !Number.isSafeInteger(value)) invalid()
      // String wire format matches scorers accepting strings/lists. Preserve
      // the original numeric spelling when there is no task envelope.
      return Object.hasOwn(contract, 'taskId') ? JSON.stringify(value) : text.trim()
    case 'records':
      if (!Array.isArray(value) || value.some(item => !item || Array.isArray(item) || typeof item !== 'object'
        || Object.keys(item).length !== contract.fields.length
        || contract.fields.some(field => !Object.hasOwn(item, field)))) invalid()
      return value
  }
}
