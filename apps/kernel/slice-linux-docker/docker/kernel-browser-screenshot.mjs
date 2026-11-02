import {captureDocumentOwner} from './browser-controller-frames.mjs';
// MP-08/MP-10/MP-11: shared screenshot policy and live document ownership.
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {captureProtectionFence} from './kernel-browser-region-protection.mjs';
import {captureProtectedPage} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument,BrowserActionError} from './browser-controller-actions.mjs';
import {timestamp} from './kernel-browser-timing.mjs';
const retired=()=>new BrowserActionError('stale_document_reference','browser capture owner retired');
const staleSurface=(data,width,height)=>{
  const header=Buffer.from(String(data).slice(0,44),'base64');
  return header.length>=24&&header.readUInt32BE(12)===0x49484452&&(header.readUInt32BE(16)!==width||header.readUInt32BE(20)!==height);
};
export async function capturePageScreenshot(host,tab,clip=null,protectedCapture=false,format='png',optimizeForSpeed=true) {
    const started = timestamp();
    const scale = host.scales.get(tab.tab_id) ?? host.viewport().device_scale_factor;
    const browser = host.browser, owner = captureDocumentOwner(browser, tab);
    const { connection, sessionId } = owner ?? await browser.resolvePageTarget(tab.target_id);
    const current = async () => {
      if (host.browser !== browser) throw retired();
      if (owner) owner.assertCurrent(); else await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    };
    await current();
    const page = host.layouts?.get(tab.tab_id) ?? geometry;
    const width = Math.round((clip?.width ?? page.width) * scale * (clip?.scale ?? 1));
    const height = Math.round((clip?.height ?? page.height) * scale * (clip?.scale ?? 1));
    const regionMasks = protectedCapture ? host.protection.targets.length || host.browser.fillTargets?.size
      ? await captureProtectionFence(connection,sessionId,tab.target_id,{...host.protection,targets:[...host.protection.targets,...(host.browser.fillTargets?.values()??[])]},{ record: reason => host.timing('region_frame_masked ' + reason, timestamp()) })
      : {async afterCapture(){return [];}} : null;
    const data = await captureProtectedPage(host.browser, tab, host.protection.values,
      host.protection.targets.filter(target => target.kind === "browser"), async () => {
        const at = timestamp();
        const sample = async () => {
          const compositor=host.compositors.get(tab.tab_id)?.source;
          if(clip)compositor?.pause();
          try{return await host.browser.inputCapture.run(connection,sessionId,async()=>{
            const shoot=()=>connection.send("Page.captureScreenshot", { format, ...(format === "jpeg" ? {quality:95} : {}), captureBeyondViewport: false, optimizeForSpeed, ...(clip ? { clip } : {}) }, sessionId);
            const first=await shoot();
            // A page that navigated to another renderer after the layout was applied can keep the old viewport: apply it again.
            if(clip||!staleSurface(first.data,width,height))return first;
            await browser.reapplyLayout(tab.target_id);
            return shoot();
          },{resumeFocus:false})}
          finally{if(clip)compositor?.resume()}
        };
        const policy=host.protection;
        const {data}=await (!protectedCapture&&!clip&&!policy.unknown&&!policy.values.length&&!policy.targets.length ? sample() : host.sampleLane(tab).run("capture",sample));
        host.timing(clip?.scale < 1 ? 'cdp_preview' : clip ? 'cdp_crop' : 'cdp_capture', at);
        return data;
      }, scale, clip);
    await current();
    const protected_regions = protectedCapture ? await regionMasks.afterCapture({ width, height }) : undefined;
    host.timing('protected_capture', started);
    if (typeof data !== "string" || data.length > 4 * 1024 * 1024) throw new Error("MD-2: frame exceeds limit");
    return { generation: host.generation, tab_id: tab.tab_id, document_id: tab.document_id, mime_type: `image/${format}`, data_base64: data, width, height, ...(protectedCapture ? { protected_regions } : {}) };
}
