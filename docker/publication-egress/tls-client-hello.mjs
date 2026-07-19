const MAX_TLS_RECORD_BYTES = 18_432
const MAX_CLIENT_HELLO_BYTES = 65_536

export function tlsClientHelloServerName(buffer) {
  const bytes = Buffer.from(buffer)
  const handshake = collectHandshake(bytes)
  if (handshake === null) return null
  if (handshake[0] !== 1) throw new Error("publication egress requires a TLS ClientHello")
  const handshakeLength = uint24(handshake, 1)
  if (handshakeLength > MAX_CLIENT_HELLO_BYTES || handshake.length < handshakeLength + 4) return null
  const body = handshake.subarray(4, 4 + handshakeLength)
  let offset = 0
  requireBytes(body, offset, 34)
  offset += 34
  const sessionLength = body[offset]
  offset += 1
  requireBytes(body, offset, sessionLength)
  offset += sessionLength
  requireBytes(body, offset, 2)
  const cipherLength = body.readUInt16BE(offset)
  offset += 2
  if (cipherLength === 0 || cipherLength % 2 !== 0) throw new Error("publication egress TLS cipher list is invalid")
  requireBytes(body, offset, cipherLength)
  offset += cipherLength
  requireBytes(body, offset, 1)
  const compressionLength = body[offset]
  offset += 1
  requireBytes(body, offset, compressionLength)
  offset += compressionLength
  requireBytes(body, offset, 2)
  const extensionsLength = body.readUInt16BE(offset)
  offset += 2
  requireBytes(body, offset, extensionsLength)
  const extensionsEnd = offset + extensionsLength
  while (offset < extensionsEnd) {
    requireBytes(body, offset, 4)
    const type = body.readUInt16BE(offset)
    const length = body.readUInt16BE(offset + 2)
    offset += 4
    requireBytes(body, offset, length)
    if (type === 0) return parseServerNameExtension(body.subarray(offset, offset + length))
    offset += length
  }
  throw new Error("publication egress TLS ClientHello omitted SNI")
}

function collectHandshake(bytes) {
  const payloads = []
  let payloadLength = 0
  let offset = 0
  let requiredHandshakeBytes = null
  while (offset < bytes.length) {
    if (bytes.length - offset < 5) return null
    if (bytes[offset] !== 22 || bytes[offset + 1] !== 3) {
      throw new Error("publication egress tunnel did not begin with a TLS handshake record")
    }
    const recordLength = bytes.readUInt16BE(offset + 3)
    if (recordLength === 0 || recordLength > MAX_TLS_RECORD_BYTES) {
      throw new Error("publication egress TLS record length is invalid")
    }
    if (bytes.length - offset - 5 < recordLength) return null
    const payload = bytes.subarray(offset + 5, offset + 5 + recordLength)
    payloads.push(payload)
    payloadLength += payload.length
    if (payloadLength > MAX_CLIENT_HELLO_BYTES + 4) throw new Error("publication egress TLS ClientHello is too large")
    const joined = Buffer.concat(payloads, payloadLength)
    if (requiredHandshakeBytes === null && joined.length >= 4) requiredHandshakeBytes = uint24(joined, 1) + 4
    if (requiredHandshakeBytes !== null && joined.length >= requiredHandshakeBytes) return joined
    offset += 5 + recordLength
  }
  return null
}

function parseServerNameExtension(extension) {
  requireBytes(extension, 0, 2)
  const listLength = extension.readUInt16BE(0)
  if (listLength !== extension.length - 2) throw new Error("publication egress TLS SNI list length is invalid")
  let offset = 2
  let serverName = null
  while (offset < extension.length) {
    requireBytes(extension, offset, 3)
    const type = extension[offset]
    const length = extension.readUInt16BE(offset + 1)
    offset += 3
    requireBytes(extension, offset, length)
    if (type === 0) {
      if (serverName !== null) throw new Error("publication egress TLS ClientHello contains duplicate host names")
      const value = extension.subarray(offset, offset + length).toString("ascii")
      if (!value || !Buffer.from(value, "ascii").equals(extension.subarray(offset, offset + length))) {
        throw new Error("publication egress TLS SNI host is invalid")
      }
      serverName = value
    }
    offset += length
  }
  if (serverName === null) throw new Error("publication egress TLS ClientHello omitted a host name")
  return serverName
}

function requireBytes(buffer, offset, length) {
  if (length < 0 || offset < 0 || offset + length > buffer.length) {
    throw new Error("publication egress TLS ClientHello is truncated")
  }
}

function uint24(buffer, offset) {
  requireBytes(buffer, offset, 3)
  return (buffer[offset] << 16) | (buffer[offset + 1] << 8) | buffer[offset + 2]
}
