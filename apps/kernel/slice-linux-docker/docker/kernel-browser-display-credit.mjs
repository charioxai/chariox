// MP-08/MP-10/MP-11: one display frame credit over the protected host seam.
// Credit-mode clients (display_next) and the kernel push pump (protocol 491)
// share this body; every frame keeps the same document/region/policy fences.
import { timestamp } from './kernel-browser-timing.mjs';
import { displayGeometry as canonicalGeometry } from './kernel-browser-geometry.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
import { assertNotCancelled, assertCurrentDocument } from './browser-controller-actions.mjs';
import { directLease, egressBytes, exactPatchLimit } from './kernel-browser-display.mjs';
import { nativeDamageTiles } from './kernel-browser-tiles.mjs';
import { MotionEncoder } from './kernel-browser-motion.mjs';
import { NativeRefiner } from './kernel-browser-refiner.mjs';
import { identicalToDelivered, nativeCreditEmpty, nativeRegionBaseCurrent, nativeRegionPending, nativeRevisionCurrent } from './kernel-browser-native-credit.mjs';
import { DisplayCapture } from './kernel-browser-display-capture.mjs';
import { DisplayDocumentPending } from './kernel-browser-display-document.mjs';
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
import {viewerDensityStill,viewerPixels,viewerStillPending} from './kernel-browser-viewer-still.mjs';

const frameKinds={video:1,stripes:2,tiles:3,png:4};
// MP-08/MP-10: a lossless patch retires the video in hand for a key (~100 KB at 1920). While a page settles after
// its key, small damage continues that chain instead and the exact repair settles it. Only then: on a page that
// never stops changing (an animation) video is always in hand, and the canvas would never become exact again.
const SETTLING_MS=1000;
export function nativePatchable(stream,sample,producer=stream.producer,now=performance.now()){
 return !(producer?.holdsVideo&&now-(stream.keyAt??-Infinity)<SETTLING_MS)&&stream.canPatchNative(sample);
}
// Row 341 (A): the exact still at the viewer's density, once per page picture (the native raster's signature), document,
// input epoch and protection. Chrome repaints while it re-rasterizes for the capture; when the same pixels come back the
// still in hand is reused, so the capture never restarts itself. A capture that fails for any reason other than a moving
// viewport falls back to the page-density exact.
export async function displayCreditViewerStill(host,tab,stream,binding){
 const c=stream.viewerStill;
 if(c&&c.source===binding.source&&c.document===binding.document&&c.pixels===binding.pixels&&c.epoch===binding.epoch&&c.policy===binding.policy)return c.still;
 const still=await viewerDensityStill({host,tab,compositor:binding.source,stream}).catch(error=>{if(error?.busy)throw error;return null;});
 // A refusal is kept too: that picture goes back to the page-density path until the page changes.
 stream.viewerStill={source:binding.source,document:binding.document,pixels:binding.pixels,epoch:binding.epoch,policy:binding.policy,still};
 return still;
}
export async function displayCredit(host, stream, command, signal) {
  // Row 487: a resize_rebind visible page waits while the kernel admits it on the resized window (bounded by its hold);
  // a refused rebind ends the pump as a retired region does.
  if(stream.rebinding&&!await stream.rebinding.done)throw new UserDomainRefusal('stale_reference','region_retired');
  const geometry=host.pageSize?.(stream.tab_id)??canonicalGeometry;
  const leased=stream.adoptLease?.(command.after_sequence);
  if(leased)host.timing.event?.('native_lease_ended',{frames:leased});
  const region=stream.regionLease;
  const regionAlive=()=>!region||(command.generation===host.generation&&stream.regionSource.valid()&&region.isAlive()&&stream.regionWindow.isCurrent());
  const retired=()=>{
    // Diagnostics (labels only): which fence retired this region's capture.
    const source=stream.regionSource;
    host.timing?.(`region_retired ${stream.regionTarget?.kind} ${command.generation!==host.generation?'generation':source.closed?'closed':!source.allowed(source.policy)?'policy':'lease'}`,timestamp());
    // A retired region is a stale binding: the viewer's pump ends at once and the client rebinds.
    return new UserDomainRefusal('stale_reference','region_retired');
  };
  if(!regionAlive())throw retired();
  const cachedSource=stream.regionSource??host.compositors.get(stream.tab_id)?.source;
  const empty=()=>nativeCreditEmpty(stream,cachedSource,host.protection,host.inputEpochs.get(stream.tab_id)??0,
      host.inputChangedAt.get(stream.tab_id)??-Infinity,command.after_sequence);
  if(empty()){
    // MP-08/MP-10/MP-11: park one serial capture credit on source/codec
    // readiness during motion instead of exchanging hundreds of empty RPCs.
    // This is negative-only scheduling; every pixel still takes full fences.
    // Input's sparse source offer also wakes this bounded wait. Push credits
    // wait in displayPushCredit instead, on every readiness source.
    if(!command.push)await stream.producer.waitReady(20,signal);
    assertNotCancelled(signal);
    if(!regionAlive())throw retired();
    if(empty())return {generation:host.generation,frame_sent:false,display_frame:null};
  }
  // MP-08/MP-10/MP-11: the source masks every sample after fresh admission.
  // The synchronous lifetime guard above admits preparation; the credit's one
  // fresh region query follows all encoding/pacing awaits before delivery.
  const native = cachedSource?.native===true&&cachedSource.attested;
  let tab;
  try { tab = region ? stream.regionSource.tab : await host.displayTarget({ tab_id: stream.tab_id, generation: command.generation },native); }
  catch(error) {
    // MP-11: pending navigation still refuses old pixels, but does not fail
    // the subscription. The event-bound loader refresh admits a later credit.
    if(!(error instanceof DisplayDocumentPending))throw error;
    return {generation:host.generation,frame_sent:false,display_frame:null};
  }
  // Recreate the closure when navigation changes the loader binding. Pixels
  // and pending repairs are then invalidated by the new document as usual.
  if (!stream.capture || stream.capture.document !== tab.document_id)
    stream.capture = new DisplayCapture((clip) => {
      return host.displayScreenshot(tab,clip);
    }, stream.device_scale_factor, () => ({ width:stream.css_width, height:stream.css_height }), host.timing);
  const { connection, sessionId } = region ? stream.regionSource : native ? host.displayDocuments.connection(tab) : await host.browser.resolvePageTarget(tab.target_id);
  const layoutAt = timestamp();
  // MP-08/MP-10: an attested window supplies the complete viewport. Layout
  // is needed only to select safe CDP crops; a retired native source falls
  // back to full protected capture when no layout was read.
  const viewport = native ? null : (await connection.send("Page.getLayoutMetrics", {}, sessionId)).cssVisualViewport;
  host.timing('capture_layout_metrics', layoutAt);
  // Native CDP clips are page rectangles. Our private damage hints are
  // viewport rectangles; use full protected capture for scroll/zoom/unknown
  // origins until that coordinate transform has separate mask/race proof.
  const nativeCropSafe = viewport?.pageX === 0 && viewport?.pageY === 0 && viewport?.scale === 1;
  const capturePolicy=host.protection;
  const epoch = host.inputEpochs.get(tab.tab_id) ?? 0;
  const compositor=await host.compositorFor(tab,stream);
  const regionRevision=compositor?.regionRevision;
  if(stream.compositorRegionRevision!==regionRevision){
    // MP-11: no pixels may leave during the metadata fence. Keep only the
    // local canvas bookkeeping until a fresh capture decides whether its
    // masks/base are identical; pending codecs/refinements always retire.
    if(nativeRegionPending(stream,compositor,tab.document_id,host.protection)){
      stream.producer?.retireUnsent(undefined,'region_pending');stream.refiner?.invalidate();
      compositor?.rejected?.('region_revision');
      return {generation:host.generation,frame_sent:false,display_frame:null};
    }
    // MP-08/MP-10/MP-11: keep only a COMPLETE exact canvas after a fresh
    // post-fence capture proves the same masks and an exact source base.
    // Unsent old frames still retire; unknown/new geometry takes a key.
    const stable=nativeRegionBaseCurrent(stream,compositor,tab.document_id,host.protection);
    if(stable){stream.producer.retireUnsent(undefined,'region_stable');stream.refiner?.invalidate();}else stream.invalidate(true,'region_revision');
    stream.compositorRegionRevision=regionRevision;
  }
  const sample=compositor?.sample();
  let source;
  // MP-08/MP-10/MP-11: an admitted native source refreshing its masks
  // must not put an exact CDP capture in front of the next input credit.
  if(compositor&&!sample&&stream.codec!=='png')return {generation:host.generation,frame_sent:false,display_frame:null};
  if(compositor&&sample&&stream.codec!=='png'){
    // Serials are source-local. Retiring a source also retires its exact
    // base, even when a newly navigated document starts at the same serial.
    if(stream.producer?.source!==compositor){stream.invalidate(true,'source');stream.document_id=null;await stream.producer?.close();stream.producer=new MotionEncoder(compositor,stream.encoder,{bitrate:stream.bitrate,areaRate:stream.whole_page===true,codec:stream.codec,independent:!stream.dependencies,stripes:stream.stripes,latest:['browser_chrome','browser_auxiliary'].includes(stream.regionTarget?.kind),shouldEncode:(sample,producer)=>{const encode=!nativePatchable(stream,sample,producer)&&!stream.shiftCandidate(sample)&&!(stream.exact&&identicalToDelivered(stream,sample));if(encode&&stream.exact)host.timing('shift_skipped_exact',timestamp());return encode;},valid:()=>!compositor.closed&&compositor.allowed(compositor.policy),timing:host.timing});}
    // Admit recovery before selecting a native patch or taking an encoded
    // packet. A lost canvas base also reoffers any skipped patchable source.
    // Once retired, empty credits must let the pending recovery key finish.
    if(stream.previous&&(stream.document_id!==tab.document_id||!stream.acceptsCredit(command.after_sequence)))stream.invalidate(true,stream.document_id!==tab.document_id?'document':'credit_lag');
    if(!stream.refiner||stream.refinerDocument!==tab.document_id){await stream.refiner?.close();stream.refiner=new NativeRefiner(async binding=>(binding.pixels!==undefined?await displayCreditViewerStill(host,tab,stream,binding):null)??(binding.native?binding.sample:region&&stream.regionSource.exactCapture?await stream.regionSource.exactCapture():await host.displayScreenshot(tab,null,false)),{now:()=>performance.now(),prepareTiles:true,timing:host.timing});stream.refinerDocument=tab.document_id;}
    // Row 341 (A): while a still at the viewer's density is pending, refinement follows the page's pixels, not the source's
    // frame serial: the clip-scale capture itself repaints the page, and a new serial for the
    // same pixels must not restart it.
    const policy=host.protection;
    const viewerPending=viewerStillPending(stream,{source:compositor,document:tab.document_id,epoch,policy,sample});
    // The pixels are read only once the page is quiet (the refiner waits for that too); a moving page keys on its serial.
    const quiet=performance.now()-Math.max(compositor.changedAt,host.inputChangedAt.get(tab.tab_id)??-Infinity)>=(stream.refiner.quietNativeMs??0);
    const pixels=viewerPending?(quiet?viewerPixels(sample):`serial:${sample.serial}`):undefined;
    const binding={source:compositor,document:tab.document_id,policy,epoch,serial:sample.serial,...(viewerPending?{pixels}:{}),...((stream.previousDensity??stream.device_scale_factor)!==stream.device_scale_factor?{whole:true}:{}),scale:stream.device_scale_factor,native:compositor.attested===true&&Boolean(sample.raw),sample,repairLimit:exactPatchLimit(stream.bitrate),encoder:stream.encoder.nativeSession,nativeDelivered:stream.encoder.nativeDeliveredRevision};
    // Always run the deadline/epoch-aware verifier before unchanged reuse.
    // A lossy JPEG fingerprint cannot rule out fine native RGB damage.
    // Row 341 (A): for a viewer above the page's density a page-density exact canvas is not final; the still at the
    // viewer's density follows it once the page is quiet.
    const nativeExact=binding.native&&stream.exact&&!viewerPending&&(sample.serial===stream.compositorSerial||stream.canPatchNative(sample));
    const exact=nativeExact?null:stream.refiner.request(binding,Math.max(compositor.changedAt,host.inputChangedAt.get(tab.tab_id)??-Infinity),()=>host.protection===policy&&(viewerPending?(quiet?viewerPixels(compositor.sample()):`serial:${compositor.sample()?.serial}`)===pixels:compositor.sample()?.serial===sample.serial)&&(host.inputEpochs.get(tab.tab_id)??0)===epoch);
    stream.creditEpoch=epoch;
    stream.producer.feedback(command.push?(command.push.congested?8:0):Math.max(0,stream.sequence-command.after_sequence));
    // MP-08/MP-10: an identical readback (protection refresh) of the
    // delivered exact canvas with the same masks needs no frame.
    if(stream.exact&&!(viewerPending&&exact?.viewer_density)&&identicalToDelivered(stream,sample)&&JSON.stringify(sample.raw[displayMaskRegions]??[])===stream.compositorMasks){
      stream.compositorSerial=sample.serial;
      return {generation:host.generation,frame_sent:false,display_frame:null};
    }
    const patchable=nativePatchable(stream,sample);
    const shift=patchable?null:stream.shiftKind(sample);
    const encoded=patchable||shift ? null : stream.producer.take();
    if(shift){
      // MP-08/MP-10: lossless scroll frame: proved moves plus WebP residuals.
      stream.producer.retireUnsent(undefined,'shift');
      const reply=await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:[],patch:true,shift,effort:stream.shiftEffort(),...(shift==='overlay'?{base:stream.compositorCommittedSerial??stream.compositorSerial}:{})});
      if(reply.shift_refused===true){
        host.timing(`shift_refused_${shift} ${/^MP-1[01]: [a-z ]{1,40}$/.test(reply.reason)?reply.reason:''}`,timestamp());
        // A refused plan holds lossless frames until exactness returns
        // and re-offers the latest sample to the video encoder.
        stream.shiftHold=true;stream.producer.retry(compositor.sample()??sample);
        compositor?.rejected?.('shift_hold');
        return {generation:host.generation,frame_sent:false,display_frame:null};
      }
      if(reply.native_packet)stream.encoder.adoptPacket(reply.native_packet);
      if(!Array.isArray(reply.native_tiles)||!Array.isArray(reply.moves)||reply.moves.length>64||reply.native_tiles.length>64||!(reply.moves.length||reply.native_tiles.length)){stream.encoder.discard?.({packet:reply.native_packet});throw Error('MP-11: native shift reply');}
      source={...sample,...reply,...(reply.moves.length?{}:{moves:undefined}),motion:false,generation:host.generation};
    }else if(patchable){
      stream.producer.retireUnsent(undefined,'patch');
      const adjacent=sample.serial===(stream.exact?(stream.compositorCommittedSerial??stream.compositorSerial):stream.compositorSerial)+1&&stream.compositorMasks===JSON.stringify(sample.raw[displayMaskRegions]??[])&&nativeDamageTiles(sample.raw,true,true)!==null;
      const patch=sample.raw.nativeExact?await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:sample.raw[displayMaskRegions]??[],patch:true,adjacent}):{native_tiles:nativeDamageTiles(sample.raw)};
      source={...sample,...patch,motion:false,generation:host.generation};
    }
    else if(encoded){source={...encoded,generation:host.generation};}
    // Row 341 (A): a viewer-density still answers the page's current pixels (the refiner matched them), so it carries the
    // current frame serial for delivery's fences.
    else if(exact&&stream.previous)source={...exact,...(exact.viewer_density?{refinement_serial:sample.serial}:{}),generation:host.generation};
    // The viewer already backs off empty credits. A second delay while
    // holding the capture gate adds latency to every pipelined slot.
    else return {generation:host.generation,frame_sent:false,display_frame:null};
  }else{
    source=await stream.capture.next({...tab,input_epoch:epoch},host.protection,stream.previous&&stream.acceptsCredit(command.after_sequence),!stream.exact||!nativeCropSafe,
      stream.codec!=='png'&&host.protection.values.length===0&&viewport?.scale===1&&Number.isFinite(viewport.pageX)&&Number.isFinite(viewport.pageY)?{x:viewport.pageX,y:viewport.pageY,width:stream.css_width,height:stream.css_height,scale:1/stream.device_scale_factor,display_motion:true}:null,
      host.scrolling.get(tab.tab_id)?.document_id===tab.document_id&&host.scrolling.get(tab.tab_id).until>performance.now());
  }
  // MP-10: only a post-dispatch native capture can claim the input burst.
  const inputAt=host.inputChangedAt.get(tab.tab_id)??-Infinity;
  source.input_triggered=Number.isFinite(inputAt)&&epoch!==stream.deliveredInputEpoch&&Number.isFinite(source.captured_ms)&&source.captured_ms>=performance.timeOrigin+inputAt;
  // MP-08/MP-10: the kernel ACK pump already bounds admitted delivery.
  // Keep this scheduling hint private; legacy display_next retains pacing.
  source.push_delivery=Boolean(command.push);
  // The commit-time document check in validate() is the ordering barrier
  // with CDP navigation events; a second pre-check adds a round trip only.
  assertNotCancelled(signal);
  const frame = await stream.frame(source, source.document_id, command.after_sequence, async () => {
    assertNotCancelled(signal);
    if(!source.motion&&(host.inputEpochs.get(tab.tab_id)??0)!==epoch)compositor?.rejected?.('epoch');
    if(region){
      if(!await (stream.regionFrameCurrent??stream.regionCurrent)()||!stream.regionWindow.isCurrent()||!compositor.valid())return false;
    }else if(native){
      if(!await host.displayDocuments.validate(tab)){compositor?.rejected?.('stale_document');return false;}
    }else await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    return !compositor?.closed&&compositor?.regionRevision===regionRevision&&(source.motion || ((host.inputEpochs.get(tab.tab_id) ?? 0) === epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&nativeRevisionCurrent(source,stream.encoder))));
  },()=>(region?region.isAlive()&&stream.regionWindow.isCurrent():!native||host.displayDocuments.current(tab))&&!compositor?.closed&&compositor?.regionRevision===regionRevision&&host.protection===capturePolicy&&(source.motion||((host.inputEpochs.get(tab.tab_id)??0)===epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&nativeRevisionCurrent(source,stream.encoder)))));
  if(frame)host.timing.event?.('frame_out',{at:source.captured_ms,source_kind:{browser_chrome:1,region:2,browser_auxiliary:3,visible_page:4,browser_window:5}[stream.regionTarget?.kind]??0,width:stream.css_width*stream.device_scale_factor,height:stream.css_height*stream.device_scale_factor,sequence:frame.sequence,kind:frameKinds[frame.kind]??0,input_triggered:source.input_triggered?1:0,input_epoch:epoch,
    damage_ready_ms:source.raw?.damage_ready_ms,input_wake_ms:source.raw?.input_wake_ms,published_ms:source.published_ms,encode_offered_ms:Number.isFinite(source.offeredAt)?performance.timeOrigin+source.offeredAt:undefined,encode_started_ms:source.encode_started_ms,encoded_ms:source.encoded_ms,native_serial:source.raw?.serial,key:frame.key?1:0,get payload_bytes(){return egressBytes(frame,stream.relay_binary)}});
  // Diagnostics (labels only): what forced this key.
  if(frame?.key)stream.keyAt=performance.now();
  if(frame?.key){host.timing?.(`display_key ${stream.keyReason??stream.encoder?.keyCause??stream.producer?.keyCause??'encoder'}`,timestamp());stream.keyReason=undefined;if(stream.producer)stream.producer.keyCause=undefined;if(stream.encoder)stream.encoder.keyCause=undefined;}
  if(frame){stream.compositorMasks=JSON.stringify(source.raw?.[displayMaskRegions]??[]);stream.compositorSerial=source.refinement_serial ?? source.serial;if(source.input_triggered)stream.deliveredInputEpoch=epoch;if(stream.exact&&source.raw?.nativeCommit){source.raw.nativeCommit(stream.encoder.nativeSession,!source.moves);stream.compositorCommittedSerial=stream.compositorSerial;}}
  compositor?.plans?.(stream.exact&&!stream.shiftHold&&stream.compositorMasks==='[]');
  return { generation: host.generation, frame_sent: frame !== null, display_frame: frame };
}

// The source's next frame, the pause or an abort, whichever comes first.
const nextSourceFrame = (source, ms, signal) => new Promise(resolve => {
  let off = () => {};
  const done = () => { clearTimeout(timer); off(); signal?.removeEventListener('abort', done); resolve(); };
  const timer = setTimeout(done, ms); off = source.subscribe(done); signal?.addEventListener('abort', done, { once: true });
});

// MP-08/MP-10: a push credit returns a frame or, after a bounded wait on
// encoder/refiner/source readiness, nothing. The kernel pump re-issues it.
export async function displayPushCredit(host, stream, command, signal, { budgetMs = 100, now = () => performance.now() } = {}) {
  if (command.push.reset) stream.resetForDelivery();
  // Row 487: the pump's credit waits for a pending resize rebind like an ordinary credit does.
  if (stream.rebinding && !await stream.rebinding.done) throw new UserDomainRefusal('stale_reference', 'region_retired');
  const deadline = now() + budgetMs;
  // P1: a parked Tab display holds the pump's credit until the Tab's next commit unparks it (or the budget ends).
  // A retired binding still ends the pump at once, so the viewer learns it before its next navigation.
  if (stream.parked) {
    await stream.waitUnparked(budgetMs);
    const region = stream.regionLease;
    if (region && !(command.generation === host.generation && stream.regionSource.valid() && region.isAlive() && stream.regionWindow.isCurrent()))
      throw new UserDomainRefusal('stale_reference', 'region_retired');
    if (stream.parked) return { generation: host.generation, frame_sent: false, display_frame: null };
  }
  for (;;) {
    const reply = await displayCredit(host, stream, command, signal);
    if (reply.frame_sent) {
      stream.pushedAt = now();
      // L3: the private grant travels beside the frame; the kernel strips it.
      const lease = reply.display_frame?.[directLease]?.();
      if (lease) { reply.direct_lease = lease; host.timing.event?.('native_lease_granted', { generation: lease.generation }); }
      return reply;
    }
    if (now() >= deadline || signal?.aborted) return reply;
    const producer = stream.producer;
    const refining = stream.refiner?.active;
    // A protected CDP capture source has no readiness signal: retry at the
    // old viewer idle-credit cadence (33 ms while active, else 100 ms).
    if (!producer) {
      const pause = Math.max(0, Math.min(now() - (stream.pushedAt ?? -Infinity) < 300 ? 33 : 100, deadline - now()));
      // Row 401 F1 (webperf-r19 A1): a screencast region (re)subscribed before its source's first frame wakes on that frame; the
      // blind pause slept through it (~100 ms of the first picture after every re-subscribe). Other sources keep the idle cadence.
      await (stream.regionScreencast ? nextSourceFrame(stream.regionSource, pause, signal) : new Promise(resolve => setTimeout(resolve, pause)));
      if (now() >= deadline) return reply; continue;
    }
    // Exact repair waits for the end of the quiet window, never past it.
    const quiet = stream.refiner?.quietNativeMs ?? 50;
    const changed = Math.max((stream.regionSource??host.compositors.get(stream.tab_id)?.source)?.changedAt ?? -Infinity, host.inputChangedAt.get(stream.tab_id) ?? -Infinity);
    const settle = Number.isFinite(changed) && now() - changed < quiet ? changed + quiet - now() + 1 : Infinity;
    const wait = Math.max(1, Math.min(deadline - now(), settle)), at = now();
    await Promise.race([producer.waitReady(wait, signal), refining ?? new Promise(() => {})]);
    // A ready queue that still yields no frame (region fence, patch hold)
    // must not spin the controller until the deadline.
    if (now() - at < 1) await new Promise(resolve => setTimeout(resolve, 4));
  }
}
