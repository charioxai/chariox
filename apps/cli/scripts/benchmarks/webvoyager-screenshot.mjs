// MP-08 / MP-10: read-only evidence capture of the existing Room Chromium.
import { writeFile } from 'node:fs/promises'
const output = process.argv[2]
if (!/^\/tmp\/benchwv-screenshot\d+\.png$/.test(output ?? '')) throw Error('MP-10 unowned screenshot path')
const info = await (await fetch('http://127.0.0.1:9222/json/version')).json()
const ws = new WebSocket(info.webSocketDebuggerUrl), pending = new Map()
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject })
let next = 0
function send(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++next, timer = setTimeout(() => { pending.delete(id); reject(Object.assign(Error('MP-10 CDP capture timeout'), { method, code: 'CDP_TIMEOUT' })) }, 10000)
    pending.set(id, { resolve, reject, timer, method })
    ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
  })
}
ws.onmessage = ({ data }) => {
  const m = JSON.parse(data), p = pending.get(m.id)
  if (!p) return
  pending.delete(m.id); clearTimeout(p.timer)
  if (m.error) p.reject(Object.assign(Error('MP-10 CDP capture failed'), { method: p.method, code: m.error.code })); else p.resolve(m.result)
}
const attached = new Set()
try {
  const targets = (await send('Target.getTargets')).targetInfos.filter(t => t.type === 'page')
  let selected
  for (const target of targets) {
    const { sessionId } = await send('Target.attachToTarget', { targetId: target.targetId, flatten: true })
    attached.add(sessionId)
    const state = await send('Runtime.evaluate', { expression: 'JSON.stringify({visible:document.visibilityState==="visible",focused:document.hasFocus()})', returnByValue: true }, sessionId)
    const flags = JSON.parse(state.result.value)
    if (flags.focused || !selected) selected = { sessionId, target, flags }
    else { await send('Target.detachFromTarget', { sessionId }); attached.delete(sessionId) }
    if (flags.focused) break
  }
  if (!selected) throw Error('MP-10 no Room page for screenshot')
  const shot = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, selected.sessionId)
  await writeFile(output, Buffer.from(shot.data, 'base64'), { mode: 0o600 })
  console.log(JSON.stringify({ mpItems: ['MP-08', 'MP-10'], browser: info.Browser, targetId: selected.target.targetId, url: selected.target.url, flags: selected.flags }))
} catch (error) {
  // No payload or raw site error message leaves the private capture process.
  console.error(JSON.stringify({ mpItems: ['MP-08', 'MP-10'], errorClass: error.name, method: error.method ?? null, code: error.code ?? null }))
  throw error
} finally {
  for (const sessionId of attached) await send('Target.detachFromTarget', { sessionId }).catch(() => {})
  for (const item of pending.values()) clearTimeout(item.timer)
  ws.close()
}
