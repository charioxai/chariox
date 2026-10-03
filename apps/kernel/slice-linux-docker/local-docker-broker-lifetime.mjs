// Install before launching Docker: startup is part of the kernel-owned lifetime.
export function watchLocalBrokerLifetime(cleanup, parentAlive) {
  let stopping = false
  const stop = () => {
    if (stopping) return
    stopping = true
    clearInterval(timer)
    try { cleanup() } finally { process.exit(0) }
  }
  const timer = setInterval(() => { if (!parentAlive()) stop() }, 1000)
  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) process.once(signal, stop)
  return stop
}
