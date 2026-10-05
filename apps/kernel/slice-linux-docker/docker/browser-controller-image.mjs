// MP-08/MP-10/MP-11: conservative whole-viewport redaction for protected Rooms.
// Re-encode pixels without PNG metadata; no raw protected frame leaves CDP.
import { deflateSync, crc32 } from "node:zlib";
export function blackPng(width, height) {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1
      || width > 4096 || height > 4096) throw new Error("invalid bounded Browser image geometry");
  const chunk = (name, body) => {
    const kind = Buffer.from(name); const size = Buffer.alloc(4); size.writeUInt32BE(body.length);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([kind, body])));
    return Buffer.concat([size, kind, body, crc]);
  };
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk("IHDR", header),
    chunk("IDAT", deflateSync(Buffer.alloc((width * 3 + 1) * height))), chunk("IEND", Buffer.alloc(0))]);
}
