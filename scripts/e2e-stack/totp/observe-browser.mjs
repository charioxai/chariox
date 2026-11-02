// MP-08 / MP-09 / MP-10 / MP-11: retain actual model-bound observations.
// Run the production host and scheduler; never record requests or Vault policy.
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const [source, root] = process.argv.slice(2)
const load = name => import(pathToFileURL(path.join(path.dirname(source), name)).href)
const { KernelBrowserHost, scheduleHostRequest } = await load('kernel-browser-host.mjs')
const { BrowserControllerStdioServer } = await load('browser-controller.mjs')
const { timedHostRequests } = await load('kernel-browser-timing.mjs')
const host = new KernelBrowserHost(root)
const directory = path.join(root, 'observations')
await mkdir(directory, { recursive: true, mode: 0o700 })
let sequence = 0
const handle = host.handle.bind(host)
host.handle = async (request, options) => {
  const reply = await handle(request, options)
  const op = request.params?.op
  if (request.method === 'host.browser' && request.params?.focused_agent === true && ['snapshot', 'screenshot'].includes(op)) {
    const text = JSON.stringify(reply)
    const absencePassed = host.protection.values.every(value => !text.includes(value))
    const prefix = path.join(directory, String(++sequence).padStart(3, '0') + '-' + op)
    await writeFile(prefix + '.jsonl', JSON.stringify({ op, absencePassed,
      filledTargets: host.browser?.fillTargets?.size ?? 0, reply: absencePassed ? reply : null }) + '\n', { mode: 0o600 })
    const frame = reply.result?.frame ?? reply.result
    if (absencePassed && frame?.mime_type === 'image/png' && typeof frame.data_base64 === 'string') {
      await writeFile(prefix + '.png', Buffer.from(frame.data_base64, 'base64'), { mode: 0o600 })
    }
  }
  return reply
}
let closing = false
const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0) }
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
await new BrowserControllerStdioServer(timedHostRequests(host,
  request => scheduleHostRequest(request, host.browser, host.viewport(), Boolean(host.chromium.desktop?.binding?.()))))
  .run().finally(() => host.stop())
process.exit(0)
