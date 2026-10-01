import { lstatSync } from "node:fs"

// A refusal whose message is safe to report: it names only public metadata.
export class LocalBrokerRefusal extends Error {}

const describe = metadata =>
  `${metadata.isSocket() ? "socket" : "non-socket"} owner ${metadata.uid} mode ${(metadata.mode & 0o777).toString(8).padStart(4, "0")}`

// The root helper binds the control socket and only then sets its final owner
// and mode. Until the deadline, a socket owned by root or by the user is still
// being published; anything else, a vanished socket, or a socket that is not
// final by the deadline is refused.
export async function awaitLocalBrokerTransport(socket, {ownerUid, helperExited, timeoutMs = 30_000,
  lstat = lstatSync, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)), now = () => performance.now()}) {
  const deadline = now() + timeoutMs
  let seen
  while (true) {
    let metadata
    try { metadata = lstat(socket) } catch (error) { if (error.code !== "ENOENT") throw error }
    if (metadata) {
      seen = metadata
      if (metadata.isSocket() && metadata.uid === ownerUid && (metadata.mode & 0o777) === 0o600) return
      if (!metadata.isSocket() || ![0, ownerUid].includes(metadata.uid)) throw new LocalBrokerRefusal(`transport refused: ${describe(metadata)}`)
    } else if (seen) throw new LocalBrokerRefusal(`transport vanished before it was published (last ${describe(seen)})`)
    const exited = helperExited()
    if (exited) throw new LocalBrokerRefusal(`helper exited before publishing the transport (${exited})`)
    if (now() >= deadline) {
      throw new LocalBrokerRefusal(seen ? `transport refused: ${describe(seen)} after ${timeoutMs} ms`
        : `transport did not appear within ${timeoutMs} ms`)
    }
    await sleep(100)
  }
}

// Until the socket is published, a helper exit or spawn failure is a refusal
// with its status; only afterwards does it end the launcher's lifetime.
export async function publishLocalBrokerTransport(child, socket, {onHelperExit, ...options}) {
  let exited
  child.on("error", error => { exited ??= `spawn failed: ${error.code ?? "unknown error"}` })
  child.once("exit", (code, signal) => { exited ??= signal ? `signal ${signal}` : `status ${code}` })
  await awaitLocalBrokerTransport(socket, {...options, helperExited: () => exited})
  if (exited) throw new LocalBrokerRefusal(`helper exited before publishing the transport (${exited})`)
  child.once("error", onHelperExit)
  child.once("exit", onHelperExit)
}
