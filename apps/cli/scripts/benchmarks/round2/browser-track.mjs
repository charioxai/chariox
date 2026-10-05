// MP-08/MP-10/MP-11: frozen Browser-only policy, separate infrastructure failures.
const tools = new Set(['status','find','text','wait_for_text','wait_for_idle','click',
  'fill','interact','submit','dialog','events','downloads','tab','history','upload'].map(name => `slice_browser_${name}`))
export function permittedBrowserTool(name,{allowNavigation=false}={}) {
  return tools.has(name) || allowNavigation && name === 'slice_open_url'
}
const httpUrl = value => { try { return ['http:','https:'].includes(new URL(value).protocol) } catch { return false } }
export function navigationAudit(actions,trace) {
  const failed = actions.filter(a => a.kind === 'navigate' && a.state === 'failed')
  if (!failed.length) return {firstFailingSeam:null,agentNavigationPolicyRejections:0}
  const calls = [...new Map(trace.map((entry,index) => [entry.id ?? index,entry])).values()]
    .filter(t => t.tool === 'slice_open_url' && ['completed','error'].includes(t.status))
  const rejected = calls.filter(t => t.status === 'error' && typeof t.input?.url === 'string'
    && !httpUrl(t.input.url) && String(t.error ?? '').includes('browser navigation URL must use HTTP or HTTPS'))
  const successful = calls.some(t => t.status === 'completed' && httpUrl(t.input?.url)
    && t.output?.browser?.action_kind === 'navigate' && httpUrl(t.output.browser.url))
  // Require complete terminal-call/action counts. Never forgive an unknown read,
  // transport or navigation failure on the basis of one unrelated policy error.
  const proven = successful && rejected.length === failed.length
    && calls.filter(t => t.status === 'error').length === failed.length
    && calls.length === actions.filter(a => a.kind === 'navigate').length
  return {firstFailingSeam:proven ? null : 'benchmark_navigation',
    agentNavigationPolicyRejections:proven ? rejected.length : 0}
}
