import assert from "node:assert/strict"
import test from "node:test"

import { tlsClientHello } from "./test-helpers.mjs"
import { tlsClientHelloServerName } from "./tls-client-hello.mjs"

test("TLS ClientHello parser extracts exact SNI across record boundaries", () => {
  assert.equal(tlsClientHelloServerName(tlsClientHello("api.openai.com")), "api.openai.com")
  assert.equal(tlsClientHelloServerName(tlsClientHello("api.anthropic.com", { splitAt: 19 })), "api.anthropic.com")
})

test("TLS ClientHello parser waits for truncation and rejects non-handshake traffic", () => {
  const hello = tlsClientHello("api.openai.com")
  assert.equal(tlsClientHelloServerName(hello.subarray(0, 12)), null)
  assert.throws(() => tlsClientHelloServerName(Buffer.from("GET / HTTP/1.1\r\n\r\n")), /TLS handshake/)
})

test("TLS ClientHello parser rejects missing SNI", () => {
  assert.throws(
    () => tlsClientHelloServerName(tlsClientHello("api.openai.com", { extensionType: 1 })),
    /omitted SNI/,
  )
})
