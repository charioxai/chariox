// MP-08/MP-10/MP-11: conservative whole-viewport redaction for protected Rooms.
// Re-encode pixels without PNG metadata; no raw protected frame leaves CDP.
import { deflateSync, crc32 } from "node:zlib";
import { assertBrowserFramesUnchanged, withBrowserFrames } from "./browser-controller-frames.mjs";
import { BrowserSnapshotError } from "./browser-controller-snapshot.mjs";

// A top Page snapshot includes local frames only. The composited screenshot
// includes isolated frames too, so inspect every renderer owned by this tab.
export async function captureProtectedBrowserImage({ connection, sessionId, targetId, documentId, viewport, protectedValues }) {
  return withBrowserFrames(connection, sessionId, targetId, documentId, async frames => {
    const hasPassword = async () => {
      let found = false;
      for (const entry of frames) {
        const snapshot = await connection.send("DOMSnapshot.captureSnapshot", { computedStyles: [] }, entry.sessionId);
        const strings = snapshot.strings ?? [];
        found ||= (snapshot.documents ?? []).some(doc => (doc.nodes?.attributes ?? []).some(pairs =>
          pairs.some((value, i) => i % 2 === 0 && strings[value]?.toLowerCase() === "type"
            && strings[pairs[i + 1]]?.toLowerCase() === "password")));
      }
      return found;
    };
    let protectedFrame = await hasPassword();
    await assertBrowserFramesUnchanged(connection, frames);
    const captured = await connection.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false, fromSurface: true }, sessionId);
    const bytes = Buffer.from(captured.data ?? "", "base64");
    if (bytes.length < 24 || !bytes.subarray(0, 8).equals(Buffer.from([137,80,78,71,13,10,26,10]))
        || bytes.readUInt32BE(16) !== viewport.css_width * viewport.device_scale_factor
        || bytes.readUInt32BE(20) !== viewport.css_height * viewport.device_scale_factor)
      throw new BrowserSnapshotError("browser_artifact_invalid", "Browser capture dimensions differ from the canonical viewport");
    // Also protect a password inserted while pixels were being captured.
    protectedFrame = await hasPassword() || protectedFrame || protectedValues.size > 0;
    await assertBrowserFramesUnchanged(connection, frames);
    return { bytes: protectedFrame ? blackPng(bytes.readUInt32BE(16), bytes.readUInt32BE(20)) : bytes,
      redaction: protectedFrame ? "full_viewport" : "none" };
  });
}

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
