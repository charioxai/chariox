// MP-08 / MP-10: bounded transport faults on the owned Web connection only.
export function createWebFaultTransport({ transformClient = value => value, delayMs } = {}) {
  let mode = 'valid', counter = 0
  const connections = new Set(), pending = new Set()
  const counts = { affected: 0, delayed: 0, dropped: 0, closed: 0 }
  const close = pair => { for (const socket of pair) { try { socket.close() } catch {} } connections.delete(pair) }
  const clear = () => { for (const timer of pending) clearTimeout(timer); pending.clear() }
  function forward(target, value, pair) {
    if (mode === 'slow-viewer' && !pair.display) { target.send(value); return }
    if (mode === 'web-disconnect') { counts.dropped++; counts.affected++; return }
    if (mode === 'valid') { target.send(value); return }
    counts.affected++
    if ((mode === 'network-loss' && ++counter % 7 === 0) || pending.size >= 32) { counts.dropped++; return }
    counts.delayed++
    const timer = setTimeout(() => { pending.delete(timer); try { target.send(value) } catch {} }, delayMs ?? (mode === 'slow-viewer' ? 1500 : 600))
    pending.add(timer)
  }
  return {
    route(socket) {
      if (mode === 'web-disconnect') { counts.closed++; counts.affected++; socket.close(); return }
      const server = socket.connectToServer(); const pair = [socket, server]
      pair.display = /^\/display\//.test(new URL(socket.url()).pathname)
      connections.add(pair)
      socket.onMessage(value => forward(server, transformClient(value), pair))
      server.onMessage(value => forward(socket, value, pair))
    },
    setMode(value) {
      if (!['valid', 'web-disconnect', 'network-loss', 'slow-viewer'].includes(value)) throw Error('unsupported Web transport fault')
      const priorMode = mode
      mode = value
      if (value === 'valid' || value === 'web-disconnect') {
        clear()
        for (const pair of [...connections]) {
          if (value === 'valid' && priorMode === 'slow-viewer' && !pair.display) continue
          counts.closed++; counts.affected++; close(pair)
        }
      }
      return { mode, ...counts }
    },
    stats: () => ({ mode, ...counts, pending: pending.size }),
    dispose() { clear(); for (const pair of [...connections]) close(pair) },
  }
}
