import assert from "node:assert/strict"
import net from "node:net"
import test from "node:test"

import {
  defaultRouteFromProc,
  gatewaySourceMatches,
  interfaceIpv4,
  parsePortRanges,
  startProviderPortBridge,
} from "../../../kernel/slice-linux-docker/docker/provider-port-bridge.mjs"

test("provider bridge parses bounded deduplicated ranges", () => {
  assert.deepEqual(parsePortRanges("46020-46022,46021,51220"), [46020, 46021, 46022, 51220])
  assert.throws(() => parsePortRanges(""), /requires at least one port/)
  assert.throws(() => parsePortRanges("46022-46020"), /invalid provider bridge port range/)
})

test("provider bridge resolves Docker interface and gateway from Linux route data", () => {
  const route = defaultRouteFromProc([
    "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT",
    "eth0 00000000 010011AC 0003 0 0 0 00000000 0 0 0",
  ].join("\n"))
  assert.deepEqual(route, { interfaceName: "eth0", gateway: "172.17.0.1" })
  assert.equal(interfaceIpv4("eth0", {
    eth0: [{ address: "172.17.0.2", family: "IPv4", internal: false }],
  }), "172.17.0.2")
  assert.equal(gatewaySourceMatches("::ffff:172.17.0.1", "172.17.0.1"), true)
  assert.equal(gatewaySourceMatches("172.17.0.3", "172.17.0.1"), false)
})

test("provider bridge forwards bytes to the same loopback port", async () => {
  const upstream = net.createServer((socket) => socket.pipe(socket))
  await listen(upstream, "127.0.0.1", 0)
  const address = upstream.address()
  assert.ok(address && typeof address !== "string")
  let bridge = null
  try {
    bridge = await startProviderPortBridge({
      ports: [address.port],
      bindHost: "::1",
      allowedSource: "::1",
    })
    const response = await roundTrip("::1", address.port, "slice-native-tui")
    assert.equal(response, "slice-native-tui")
  } finally {
    await bridge?.close()
    await close(upstream)
  }
})

function listen(server, host, port) {
  return new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, host, () => resolve())
  })
}

function close(server) {
  return new Promise((resolve) => server.close(resolve))
}

function roundTrip(host, port, payload) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port })
    let response = ""
    socket.setEncoding("utf8")
    socket.once("error", reject)
    socket.on("data", (chunk) => {
      response += chunk
      if (response.length >= payload.length) socket.end()
    })
    socket.once("close", () => resolve(response))
    socket.once("connect", () => socket.write(payload))
  })
}
