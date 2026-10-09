import { withBrowserFrames, assertBrowserFramesUnchanged } from './browser-controller-frames.mjs';
// MP-08/MP-10/MP-11: recorded Vault fill targets protect screenshot artifacts.
// Re-encode pixels without PNG metadata; no raw protected frame leaves CDP.
import { deflateSync, crc32 } from "node:zlib";
import { BrowserSnapshotError } from "./browser-controller-snapshot.mjs";

// MP-08/MP-11: screenshot artifacts use the same exact fill targets as video.
export async function captureProtectedBrowserImage({connection,sessionId,targetId,documentId,viewport,protectedValues,fillTargets=[]}) {
  return withBrowserFrames(connection,sessionId,targetId,documentId,async frames=>{
  const {captureProtectedPage}=await import('./kernel-browser-pixels.mjs');
  const browser={fillTargets:new Map(fillTargets.map((t,i)=>[i,t])),async resolvePageTarget(){return {connection,sessionId};}};
  const data=await captureProtectedPage(browser,{target_id:targetId,document_id:documentId},[...protectedValues],fillTargets,async()=>{
    const captured=await connection.send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false,fromSurface:true},sessionId);
    return captured.data;
  },viewport.device_scale_factor);
  const bytes=Buffer.from(data,'base64');
  if(bytes.length<24||bytes.readUInt32BE(16)!==viewport.css_width*viewport.device_scale_factor||bytes.readUInt32BE(20)!==viewport.css_height*viewport.device_scale_factor)throw new BrowserSnapshotError('browser_artifact_invalid','Browser capture dimensions differ from the canonical viewport');
  await assertBrowserFramesUnchanged(connection,frames);
  return {bytes,redaction:fillTargets.length?'fill_targets':'none'};
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
