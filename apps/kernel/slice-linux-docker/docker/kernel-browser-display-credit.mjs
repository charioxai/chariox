// MP-08/MP-10/MP-11: one display frame credit over the protected host seam.
// Credit-mode clients (display_next) and the kernel push pump (protocol 475)
// share this body; every frame keeps the same document/region/policy fences.
import { timestamp } from './kernel-browser-timing.mjs';
import { displayGeometry as geometry } from './kernel-browser-geometry.mjs';
import { displayMaskRegions } from './kernel-browser-pixels.mjs';
import { assertNotCancelled, assertCurrentDocument } from './browser-controller-actions.mjs';
import { exactPatchLimit } from './kernel-browser-display.mjs';
import { nativeDamageTiles } from './kernel-browser-tiles.mjs';
import { MotionEncoder } from './kernel-browser-motion.mjs';
import { NativeRefiner } from './kernel-browser-refiner.mjs';
import { identicalToDelivered, nativeCreditEmpty, nativeRegionBaseCurrent, nativeRegionPending } from './kernel-browser-native-credit.mjs';
import { DisplayCapture } from './kernel-browser-display-capture.mjs';

export async function displayCredit(host, stream, command, signal) {
  const cachedSource=host.compositors.get(stream.tab_id)?.source;
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
    if(empty())return {generation:host.generation,frame_sent:false,display_frame:null};
  }
  const tab = await host.displayTarget({ tab_id: stream.tab_id, generation: command.generation });
  // Recreate the closure when navigation changes the loader binding. Pixels
  // and pending repairs are then invalidated by the new document as usual.
  if (!stream.capture || stream.capture.document !== tab.document_id)
    stream.capture = new DisplayCapture((clip) => {
      return host.displayScreenshot(tab,clip);
    }, stream.device_scale_factor, host.timing);
  const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  const layoutAt = timestamp();
  // MP-08/MP-10: an attested window supplies the complete viewport. Layout
  // is needed only to select safe CDP crops; a retired native source falls
  // back to full protected capture when no layout was read.
  const viewport = cachedSource?.attested ? null : (await connection.send("Page.getLayoutMetrics", {}, sessionId)).cssVisualViewport;
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
      stream.producer?.retireUnsent();stream.refiner?.invalidate();
      return {generation:host.generation,frame_sent:false,display_frame:null};
    }
    // MP-08/MP-10/MP-11: keep only a COMPLETE exact canvas after a fresh
    // post-fence capture proves the same masks and an exact source base.
    // Unsent old frames still retire; unknown/new geometry takes a key.
    const stable=nativeRegionBaseCurrent(stream,compositor,tab.document_id,host.protection);
    if(stable){stream.producer.retireUnsent();stream.refiner?.invalidate();}else stream.invalidate();
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
    if(stream.producer?.source!==compositor){stream.invalidate();stream.document_id=null;await stream.producer?.close();stream.producer=new MotionEncoder(compositor,stream.encoder,{bitrate:stream.bitrate,codec:stream.codec,independent:!stream.dependencies,stripes:stream.stripes,shouldEncode:sample=>{const encode=!stream.canPatchNative(sample)&&!stream.shiftCandidate(sample)&&!(stream.exact&&identicalToDelivered(stream,sample));if(encode&&stream.exact)host.timing('shift_skipped_exact',timestamp());return encode;},valid:()=>!compositor.closed&&compositor.allowed(compositor.policy),timing:host.timing});}
    // Admit recovery before selecting a native patch or taking an encoded
    // packet. A lost canvas base also reoffers any skipped patchable source.
    // Once retired, empty credits must let the pending recovery key finish.
    if(stream.previous&&(stream.document_id!==tab.document_id||!stream.acceptsCredit(command.after_sequence)))stream.invalidate();
    if(!stream.refiner||stream.refinerDocument!==tab.document_id){await stream.refiner?.close();stream.refiner=new NativeRefiner(binding=>binding.native?binding.sample:host.displayScreenshot(tab,null,false),{now:()=>performance.now(),prepareTiles:true,timing:host.timing});stream.refinerDocument=tab.document_id;}
    const policy=host.protection;
    const binding={source:compositor,document:tab.document_id,policy,epoch,serial:sample.serial,scale:stream.device_scale_factor,native:compositor.attested===true&&Boolean(sample.raw),sample,repairLimit:exactPatchLimit(stream.bitrate),encoder:stream.encoder.nativeSession,nativeDelivered:stream.encoder.nativeDeliveredRevision};
    // Always run the deadline/epoch-aware verifier before unchanged reuse.
    // A lossy JPEG fingerprint cannot rule out fine native RGB damage.
    const nativeExact=binding.native&&stream.exact&&(sample.serial===stream.compositorSerial||stream.canPatchNative(sample));
    const exact=nativeExact?null:stream.refiner.request(binding,Math.max(compositor.changedAt,host.inputChangedAt.get(tab.tab_id)??-Infinity),()=>host.protection===policy&&compositor.sample()?.serial===sample.serial&&(host.inputEpochs.get(tab.tab_id)??0)===epoch);
    stream.creditEpoch=epoch;
    stream.producer.feedback(command.push?(command.push.congested?8:0):Math.max(0,stream.sequence-command.after_sequence));
    // MP-08/MP-10: an identical readback (protection refresh) of the
    // delivered exact canvas with the same masks needs no frame.
    if(stream.exact&&identicalToDelivered(stream,sample)&&JSON.stringify(sample.raw[displayMaskRegions]??[])===stream.compositorMasks){
      stream.compositorSerial=sample.serial;
      return {generation:host.generation,frame_sent:false,display_frame:null};
    }
    const patchable=stream.canPatchNative(sample);
    const shift=patchable?null:stream.shiftKind(sample);
    const encoded=patchable||shift ? null : stream.producer.take();
    if(shift){
      // MP-08/MP-10: lossless scroll frame: proved moves plus WebP residuals.
      stream.producer.retireUnsent();
      const reply=await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:[],patch:true,shift,effort:stream.shiftEffort(),...(shift==='overlay'?{base:stream.compositorCommittedSerial??stream.compositorSerial}:{})});
      if(reply.shift_refused===true){
        host.timing(`shift_refused_${shift} ${/^MP-1[01]: [a-z ]{1,40}$/.test(reply.reason)?reply.reason:''}`,timestamp());
        // A refused plan holds lossless frames until exactness returns
        // and re-offers the latest sample to the video encoder.
        stream.shiftHold=true;stream.producer.retry(compositor.sample()??sample);
        return {generation:host.generation,frame_sent:false,display_frame:null};
      }
      if(reply.native_packet)stream.encoder.adoptPacket(reply.native_packet);
      if(!Array.isArray(reply.native_tiles)||!Array.isArray(reply.moves)||reply.moves.length>64||reply.native_tiles.length>64||!(reply.moves.length||reply.native_tiles.length)){stream.encoder.discard?.({packet:reply.native_packet});throw Error('MP-11: native shift reply');}
      source={...sample,...reply,...(reply.moves.length?{}:{moves:undefined}),motion:false,generation:host.generation};
    }else if(patchable){
      stream.producer.retireUnsent();
      const adjacent=sample.serial===(stream.exact?(stream.compositorCommittedSerial??stream.compositorSerial):stream.compositorSerial)+1&&stream.compositorMasks===JSON.stringify(sample.raw[displayMaskRegions]??[])&&nativeDamageTiles(sample.raw,true,true)!==null;
      const patch=sample.raw.nativeExact?await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:sample.raw[displayMaskRegions]??[],patch:true,adjacent}):{native_tiles:nativeDamageTiles(sample.raw)};
      source={...sample,...patch,motion:false,generation:host.generation};
    }
    else if(encoded){source={...encoded,generation:host.generation};}
    else if(exact&&stream.previous)source={...exact,generation:host.generation};
    // The viewer already backs off empty credits. A second delay while
    // holding the capture gate adds latency to every pipelined slot.
    else return {generation:host.generation,frame_sent:false,display_frame:null};
  }else{
    source=await stream.capture.next({...tab,input_epoch:epoch},host.protection,stream.previous&&stream.acceptsCredit(command.after_sequence),!stream.exact||!nativeCropSafe,
      stream.codec!=='png'&&host.protection.values.length===0&&viewport?.scale===1&&Number.isFinite(viewport.pageX)&&Number.isFinite(viewport.pageY)?{x:viewport.pageX,y:viewport.pageY,width:geometry.width,height:geometry.height,scale:1/stream.device_scale_factor,display_motion:true}:null,
      host.scrolling.get(tab.tab_id)?.document_id===tab.document_id&&host.scrolling.get(tab.tab_id).until>performance.now());
  }
  // MP-10: only a post-dispatch native capture can claim the input burst.
  const inputAt=host.inputChangedAt.get(tab.tab_id)??-Infinity;
  source.input_triggered=Number.isFinite(inputAt)&&epoch!==stream.deliveredInputEpoch&&Number.isFinite(source.captured_ms)&&source.captured_ms>=performance.timeOrigin+inputAt;
  // The commit-time document check in validate() is the ordering barrier
  // with CDP navigation events; a second pre-check adds a round trip only.
  assertNotCancelled(signal);
  const frame = await stream.frame(source, source.document_id, command.after_sequence, async () => {
    assertNotCancelled(signal);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    return !compositor?.closed&&compositor?.regionRevision===regionRevision&&(source.motion || ((host.inputEpochs.get(tab.tab_id) ?? 0) === epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&(source.native_revision===undefined||source.native_revision===stream.encoder.nativeRevision))));
  },()=>!compositor?.closed&&compositor?.regionRevision===regionRevision&&host.protection===capturePolicy&&(source.motion||((host.inputEpochs.get(tab.tab_id)??0)===epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&(source.native_revision===undefined||source.native_revision===stream.encoder.nativeRevision)))));
  if(frame){stream.compositorMasks=JSON.stringify(source.raw?.[displayMaskRegions]??[]);stream.compositorSerial=source.refinement_serial ?? source.serial;if(source.input_triggered)stream.deliveredInputEpoch=epoch;if(stream.exact&&source.raw?.nativeCommit){source.raw.nativeCommit(stream.encoder.nativeSession,!source.moves);stream.compositorCommittedSerial=stream.compositorSerial;}}
  compositor?.plans?.(stream.exact&&!stream.shiftHold&&stream.compositorMasks==='[]');
  return { generation: host.generation, frame_sent: frame !== null, display_frame: frame };
}

// MP-08/MP-10: a push credit returns a frame or, after a bounded wait on
// encoder/refiner/source readiness, nothing. The kernel pump re-issues it.
export async function displayPushCredit(host, stream, command, signal, { budgetMs = 100, now = () => performance.now() } = {}) {
  if (command.push.reset) stream.invalidate();
  const deadline = now() + budgetMs;
  for (;;) {
    const reply = await displayCredit(host, stream, command, signal);
    if (reply.frame_sent) stream.pushedAt = now();
    if (reply.frame_sent || now() >= deadline || signal?.aborted) return reply;
    const producer = stream.producer;
    const refining = stream.refiner?.active;
    // A protected CDP capture source has no readiness signal: retry at the
    // old viewer idle-credit cadence (33 ms while active, else 100 ms).
    if (!producer) { await new Promise(resolve => setTimeout(resolve, Math.max(0, Math.min(now() - (stream.pushedAt ?? -Infinity) < 300 ? 33 : 100, deadline - now())))); if (now() >= deadline) return reply; continue; }
    // Exact repair waits for the end of the quiet window, never past it.
    const quiet = stream.refiner?.quietNativeMs ?? 50;
    const changed = Math.max(host.compositors.get(stream.tab_id)?.source?.changedAt ?? -Infinity, host.inputChangedAt.get(stream.tab_id) ?? -Infinity);
    const settle = Number.isFinite(changed) && now() - changed < quiet ? changed + quiet - now() + 1 : Infinity;
    const wait = Math.max(1, Math.min(deadline - now(), settle)), at = now();
    await Promise.race([producer.waitReady(wait, signal), refining ?? new Promise(() => {})]);
    // A ready queue that still yields no frame (region fence, patch hold)
    // must not spin the controller until the deadline.
    if (now() - at < 1) await new Promise(resolve => setTimeout(resolve, 4));
  }
}