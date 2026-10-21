// MP-08 / MP-10 / MP-11: lab transport instrumentation; never inspect or log runtime payloads.
import http from 'node:http'
import net from 'node:net'
import { appendFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { isAbsolute } from 'node:path'

export function timingMux({ relay, baseline, record }) {
  for (const port of [relay, baseline]) {
    if (!Number.isSafeInteger(port) || port < 1024 || port > 65535) throw Error('MP-11: owned loopback port required')
  }
  const peers = new Set()
  let serial = 0
  const at = () => performance.timeOrigin + performance.now()
  const server = http.createServer((req, res) => {
    const upstream = http.request({ host: '127.0.0.1', port: baseline, path: req.url, method: req.method, headers: req.headers }, reply => {
      res.writeHead(reply.statusCode, reply.headers)
      reply.pipe(res)
    })
    upstream.on('error', () => { res.writeHead(502); res.end() })
    req.pipe(upstream)
  })
  server.on('upgrade', (req, client, head) => {
    const path = req.url.split('?')[0] === '/api/websockets' ? 1 : 0
    const ua = /Mozilla/.test(req.headers['user-agent'] ?? '') ? 'browser' : 'native'
    const upstream = net.connect(path === 0 ? relay : baseline, '127.0.0.1')
    for (const socket of [client, upstream]) {
      peers.add(socket)
      socket.setNoDelay(true)
      socket.on('error', () => { client.destroy(); upstream.destroy() })
      socket.once('close', () => { peers.delete(socket); client.destroy(); upstream.destroy() })
    }
    upstream.once('connect', () => {
      let request = `${req.method} ${req.url} HTTP/${req.httpVersion}\r\n`
      for (let i = 0; i < req.rawHeaders.length; i += 2) request += req.rawHeaders[i] + ': ' + req.rawHeaders[i + 1] + '\r\n'
      upstream.write(request + '\r\n')
      if (head.length) upstream.write(head)
      const conn = ++serial
      record({ at: at(), conn, path, ua, open: true })
      const timing = (from, to, dir) => from.on('data', chunk => {
        const stamp = at()
        to.write(chunk)
        record({ at: stamp, conn, dir, bytes: chunk.length })
      })
      timing(client, upstream, 'up')
      timing(upstream, client, 'down')
      client.once('close', () => record({ at: at(), conn, closed: true }))
    })
  })
  return { server, close() { for (const peer of peers) peer.destroy(); return new Promise(resolve => server.close(resolve)) } }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [listen, relay, baseline] = process.argv.slice(2, 5).map(Number)
  const log = process.argv[5]
  if (!Number.isSafeInteger(listen) || listen < 1024 || listen > 65535 || !log || !isAbsolute(log)) throw Error('MP-11: owned port and absolute external log path required')
  let rows = []
  const flush = () => { if (rows.length) { appendFileSync(log, rows.join('\n') + '\n', { mode: 0o600 }); rows = [] } }
  const mux = timingMux({ relay, baseline, record(row) { rows.push(JSON.stringify(row)); if (rows.length >= 256) flush() } })
  const timer = setInterval(flush, 1000)
  await new Promise(resolve => mux.server.listen(listen, '127.0.0.1', resolve))
  for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, async () => { clearInterval(timer); await mux.close(); flush() })
}
