import test from "node:test"
import assert from "node:assert/strict"
import {createECDH} from "node:crypto"
import {AgentTriggerIngressClient, isSecureRelayUrl, relayKeypairFromPrivateKey} from "./agent-trigger-ingress-client.js"

test("a relay must be wss://, or ws:// only on this machine", () => {
  for (const url of ["wss://relay.example", "ws://127.0.0.1:9", "ws://localhost:9", "ws://[::1]:9"]) assert.ok(isSecureRelayUrl(url), url)
  for (const url of ["ws://relay.example", "ws://127.0.0.1.example.com", "http://127.0.0.1", "relay"]) assert.ok(!isSecureRelayUrl(url), url)
})

test("connect refuses a plain-text remote relay before opening a socket", async () => {
  const grant = {relayUrl: "ws://relay.example", relayToken: "t", relayTokenExpiresAt: new Date(Date.now() + 60_000), target: {daemonId: "kernel", kernelRef: "kernel"}}
  await assert.rejects(AgentTriggerIngressClient.connect("kernel", async () => grant, "https://ingress.example"), /agent relay target unavailable/)
})

test("a stored private key gives the same public key and thumbprint a paired terminal presents", async () => {
  const ecdh = createECDH("prime256v1"); const publicKey = ecdh.generateKeys("base64")
  const keypair = await relayKeypairFromPrivateKey(ecdh.getPrivateKey())
  assert.equal(keypair.publicKeyBase64, publicKey)
  assert.equal(keypair.privateKey.algorithm.name, "ECDH")
  const first = await AgentTriggerIngressClient.connect("kernel", async thumbprint => { throw Object.assign(Error("stop"), {thumbprint}) }, "https://ingress.example", keypair).catch((error: {thumbprint?: string}) => error)
  const { createHash } = await import("node:crypto")
  assert.equal((first as {thumbprint?: string}).thumbprint, createHash("sha256").update(publicKey, "utf8").digest("hex"))
})
