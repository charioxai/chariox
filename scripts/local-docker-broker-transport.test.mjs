import test from "node:test"
import assert from "node:assert/strict"
import { chmod, mkdtemp, rm } from "node:fs/promises"
import { createServer } from "node:net"
import { once } from "node:events"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { LocalBrokerRefusal, awaitLocalBrokerTransport } from "../apps/kernel/slice-linux-docker/local-docker-broker-transport.mjs"

const OWNER = 1000
const entry = (uid, mode, socket = true) => ({uid, mode, isSocket: () => socket})
const missing = () => { throw Object.assign(new Error("missing"), {code: "ENOENT"}) }

// Replays lstat results (the last one repeats) on a fake clock.
function fixture(states, {exited = () => false} = {}) {
  let clock = 0
  let index = 0
  const options = {ownerUid: OWNER, helperExited: exited, timeoutMs: 1000, now: () => clock,
    sleep: async ms => { clock += ms },
    lstat: () => { const state = states[Math.min(index++, states.length - 1)]; return state === undefined ? missing() : state }}
  return {options, elapsed: () => clock}
}

test("a socket the helper creates as root and hands over late is published", async () => {
  const {options, elapsed} = fixture([undefined, entry(0, 0o755), entry(0, 0o755), entry(OWNER, 0o755), entry(OWNER, 0o600)])
  await awaitLocalBrokerTransport("/fixture/control.sock", options)
  assert.equal(elapsed(), 400)
})

test("a real socket whose mode becomes final late is published", async () => {
  const directory = await mkdtemp(join(tmpdir(), "chariox-broker-transport-"))
  const socket = join(directory, "control.sock")
  const server = createServer()
  try {
    server.listen(socket)
    await once(server, "listening")
    await chmod(socket, 0o755)
    setTimeout(() => chmod(socket, 0o600), 250)
    await awaitLocalBrokerTransport(socket, {ownerUid: process.getuid(), helperExited: () => false, timeoutMs: 5000})
  } finally {
    server.close()
    await rm(directory, {recursive: true, force: true})
  }
})

for (const [label, state, reason] of [
  ["root-owned", entry(0, 0o755), /^transport refused: socket owner 0 mode 0755 after 1000 ms$/],
  ["wrong mode", entry(OWNER, 0o660), /^transport refused: socket owner 1000 mode 0660 after 1000 ms$/],
]) {
  test(`a socket left ${label} at the deadline is refused with its owner and mode`, async () => {
    const {options, elapsed} = fixture([state])
    await assert.rejects(awaitLocalBrokerTransport("/fixture/control.sock", options),
      error => error instanceof LocalBrokerRefusal && reason.test(error.message))
    assert.equal(elapsed(), 1000)
  })
}

for (const [label, state, reason] of [
  ["foreign owner", entry(4242, 0o600), /owner 4242 mode 0600/],
  ["non-socket", entry(OWNER, 0o600, false), /non-socket owner 1000/],
]) {
  test(`a ${label} transport is refused at once`, async () => {
    const {options, elapsed} = fixture([state])
    await assert.rejects(awaitLocalBrokerTransport("/fixture/control.sock", options),
      error => error instanceof LocalBrokerRefusal && reason.test(error.message))
    assert.equal(elapsed(), 0)
  })
}

test("a vanished socket, an exited helper and a missing socket are refused", async () => {
  const cases = [
    [fixture([entry(0, 0o755), undefined]), /^transport vanished before it was published \(last socket owner 0 mode 0755\)$/],
    [fixture([undefined], {exited: () => true}), /helper exited/],
    [fixture([undefined]), /did not appear within 1000 ms/],
  ]
  for (const [{options}, reason] of cases) {
    await assert.rejects(awaitLocalBrokerTransport("/fixture/control.sock", options),
      error => error instanceof LocalBrokerRefusal && reason.test(error.message))
  }
})
