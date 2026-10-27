export function tlsClientHello(host, { extensionType = 0, splitAt = null } = {}) {
  const name = Buffer.from(host, "ascii")
  const serverName = Buffer.concat([
    Buffer.from([0, name.length >> 8, name.length & 0xff]),
    name,
  ])
  const serverNameList = Buffer.concat([
    Buffer.from([serverName.length >> 8, serverName.length & 0xff]),
    serverName,
  ])
  const extension = Buffer.concat([
    Buffer.from([extensionType >> 8, extensionType & 0xff, serverNameList.length >> 8, serverNameList.length & 0xff]),
    serverNameList,
  ])
  const body = Buffer.concat([
    Buffer.from([3, 3]),
    Buffer.alloc(32, 7),
    Buffer.from([0]),
    Buffer.from([0, 2, 0x13, 0x01]),
    Buffer.from([1, 0]),
    Buffer.from([extension.length >> 8, extension.length & 0xff]),
    extension,
  ])
  const handshake = Buffer.concat([
    Buffer.from([1, (body.length >> 16) & 0xff, (body.length >> 8) & 0xff, body.length & 0xff]),
    body,
  ])
  if (splitAt === null) return tlsRecord(handshake)
  return Buffer.concat([
    tlsRecord(handshake.subarray(0, splitAt)),
    tlsRecord(handshake.subarray(splitAt)),
  ])
}

function tlsRecord(payload) {
  return Buffer.concat([
    Buffer.from([22, 3, 1, payload.length >> 8, payload.length & 0xff]),
    payload,
  ])
}
