import { spawn } from 'node:child_process'

const PROVIDER_ACCOUNT_PATTERNS = [
  /insufficient balance/i,
  /no credits?/i,
  /manage your billing/i,
  /billing (hard )?limit/i,
  /insufficient_quota/i,
  /rate limit/i,
  /usage limit/i,
]

const PROVIDER_AUTH_PATTERNS = [
  /\b(?:http|status(?: code)?|response|token refresh failed:?)\s*401\b/i,
  /\b401\s+unauthori[sz]ed\b/i,
  /unauthori[sz]ed/i,
  /authentication/i,
  /not logged in/i,
  /login required/i,
  /token refresh failed/i,
]

export function classifyDrillChildFailure(text) {
  if (PROVIDER_ACCOUNT_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'provider-account'
  }
  if (PROVIDER_AUTH_PATTERNS.some((pattern) => pattern.test(text))) {
    return 'provider-auth'
  }
  if (/provider_error|OpenCode error|Codex error|Claude error/i.test(text)) {
    return 'provider-error'
  }
  return 'child-process'
}

export function formatDrillChildFailure(label, code, signal, stdout, stderr) {
  const combined = `${stdout}\n${stderr}`.trim()
  const classification = classifyDrillChildFailure(combined)
  const exit = signal ? `signal ${signal}` : `code ${code}`
  const tail = combined.split('\n').slice(-40).join('\n').trim()
  return [
    `${label} child failed with ${exit} (${classification})`,
    classification === 'provider-account'
      ? 'Provider account/billing blocked validation before the remote runtime behavior could be proven.'
      : null,
    classification === 'provider-auth'
      ? 'Provider authentication blocked validation before the remote runtime behavior could be proven.'
      : null,
    tail ? `child output tail:\n${tail}` : null,
  ].filter(Boolean).join('\n')
}

export async function runNodeDrillChild(args, cwd, { label }) {
  return await new Promise((resolve, reject) => {
    const child = spawn('node', args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      process.stdout.write(chunk)
      stdout += chunk.toString()
    })
    child.stderr.on('data', (chunk) => {
      process.stderr.write(chunk)
      stderr += chunk.toString()
    })
    child.on('exit', (code, signal) => {
      if (code === 0) resolve(stdout)
      else reject(new Error(formatDrillChildFailure(label, code, signal, stdout, stderr)))
    })
    child.on('error', reject)
  })
}
