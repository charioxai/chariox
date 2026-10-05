// MP-08/MP-10/MP-11: diagnostic classification and admission pauses, never a scoring rule.
import { createHash } from 'node:crypto'
import { sanitizeDrillMetadata } from '../../lib/drill-secrets.mjs'

export function providerFailureFlags(value) {
  const text = typeof value === 'string' ? value : JSON.stringify(value)
  return {
    usageExhausted: /usage.{0,40}exhausted|usage.limit|rate.limit|quota|429|out.of.credit|limit.reached/i.test(text),
    weeklyWindow: /weekly/i.test(text),
    unauthorized: /401|unauthorized|refresh.token.reused|authentication.failed|invalid.api.key/i.test(text),
  }
}

export function nativeFailureEvidence({ output, exitCode, abort }) {
  const errorEvents = output.split('\n').flatMap(line => {
    try {
      const event = JSON.parse(line)
      return ['error', 'turn.failed'].includes(event.type) ? [sanitizeDrillMetadata(event)] : []
    } catch { return [] }
  })
  return { mpItems: ['MP-08', 'MP-10', 'MP-11'], available: false, exitCode, abort: abort ?? null,
    stdoutBytes: Buffer.byteLength(output), stdoutSha256: createHash('sha256').update(output).digest('hex'),
    errorEvents, ...providerFailureFlags(errorEvents) }
}

export function requiresCampaignPause(row) {
  return !row.cleanupValid || !!row.providerUnauthorized || !!row.providerError
    || ['failed', 'cancelled'].includes(row.turnLifecycle)
    || !!row.judgeFailure || !!row.providerUsageExhausted
}
