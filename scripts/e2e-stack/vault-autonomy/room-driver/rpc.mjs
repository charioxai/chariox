// MP-08 / MP-09 / MP-10 / MP-11: owner IPC for one named kernel (a local drill kernel, same host).
import {readFile} from 'node:fs/promises'
import {randomUUID} from 'node:crypto'
import {createRequire} from 'node:module'

const kernelRoot = process.env.ROOM_KERNEL_ROOT
if (!kernelRoot) throw Error('Set ROOM_KERNEL_ROOT to the kernel directory that holds ports.json and home/')
const WS = createRequire(process.env.ROOM_WS_FROM ?? import.meta.url)('ws')

export async function connect() {
  const ports = JSON.parse(await readFile(`${kernelRoot}/ports.json`, 'utf8'))
  const credential = (await readFile(`${kernelRoot}/home/state/kernel-local-auth/${ports.kernel}.token`, 'utf8')).trim()
  const socket = new WS(`ws://127.0.0.1:${ports.kernel}/kernel`, {headers: {authorization: `Bearer ${credential}`}})
  await new Promise((resolve, reject) => { socket.once('open', resolve); socket.once('error', reject) })
  const pending = new Map()
  socket.on('message', data => {
    let frame
    try { frame = JSON.parse(String(data)) } catch { return }
    const entry = pending.get(frame.request_id)
    if (!entry) return
    pending.delete(frame.request_id)
    clearTimeout(entry.timer)
    frame.error ? entry.reject(new Error(`${entry.kind}: ${frame.error.message}`)) : entry.resolve(frame.response)
  })
  socket.on('close', () => {
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(Error('kernel disconnected')) }
    pending.clear()
  })
  const send = request => new Promise((resolve, reject) => {
    const id = randomUUID(), kind = Object.keys(request)[0]
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`${kind} timed out`)) }, 60000)
    pending.set(id, {resolve, reject, timer, kind})
    socket.send(JSON.stringify({type: 'request', request_id: id, request}))
  })
  return {send, close: () => socket.close()}
}
