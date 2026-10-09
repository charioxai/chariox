// MP-08/MP-10/MP-11: negative-only readiness; never authorize or emit pixels.
// The caller already checked scope/generation and renewed the admitted stream.
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
export function nativeRegionPending(stream,source,document,policy){
 return Boolean(source?.attested&&typeof source.valid==='function'&&source.valid()&&source.policy===policy&&source.allowed(policy)&&
  source.tab?.tab_id===stream.tab_id&&source.tab.document_id===document&&!source.sample());
}
export function nativeRegionBaseCurrent(stream,source,document,policy){
 const sample=source?.sample();
 return Boolean(stream.exact&&stream.document_id===document&&stream.producer?.source===source&&
  source.attested&&typeof source.valid==='function'&&source.valid()&&source.policy===policy&&source.allowed(policy)&&
  sample?.document_id===document&&sample.tab_id===stream.tab_id&&sample.raw?.nativeExact&&
  (sample.raw.base_serial===stream.compositorSerial||identicalToDelivered(stream,sample))&&
  JSON.stringify(sample.raw[displayMaskRegions]??[])===stream.compositorMasks);
}
// MP-08/MP-10/MP-11: the capture proved this readback byte-identical to the
// previous one, which is the delivered frame when the serials are adjacent.
export function identicalToDelivered(stream,sample){
 return sample?.raw?.identical===true&&Number.isSafeInteger(stream.compositorSerial)&&sample.serial===stream.compositorSerial+1;
}
export function nativeCreditEmpty(stream,source,policy,epoch,changedAt,after,now=performance.now()){
 const producer=stream.producer,refiner=stream.refiner;
 const quiet=refiner?.quietNativeMs??refiner?.quietMs;
 if(!source?.attested||source.closed||source.policy!==policy||!source.valid?.()||!source.allowed(policy)||
    !stream.previous||stream.repair||!stream.acceptsCredit(after)||stream.creditEpoch!==epoch||
    producer?.source!==source||producer.failure||producer.frames.length||
    !refiner||refiner.closed||refiner.failure)return false;
 const sample=source.sample();
 if(!sample?.raw||sample.document_id!==stream.document_id||sample.tab_id!==stream.tab_id)return false;
 if(producer.active||producer.pending){
  // MP-08/MP-10/MP-11: no packet exists until the private encoder replies.
  // A fresh motion sample otherwise repeats reconcile/layout CDP work for
  // every empty pipelined slot. Ready exact patches and retired mask bindings
  // still take the full path; this shortcut can only return an empty credit.
  return sample.serial>stream.compositorSerial&&stream.compositorRegionRevision===source.regionRevision&&
   now-Math.max(source.changedAt,changedAt)<quiet&&stream.canPatchNative?.(sample)===false&&!stream.shiftCandidate?.(sample);
 }
 if(sample.serial!==stream.compositorSerial)return false;
 if(now-Math.max(source.changedAt,changedAt)<quiet)return true;
 // MP-08/MP-10/MP-11: complete exact bytes remain exact after every
 // admitted contiguous native tile patch. No lossy-source equality inference.
 return Boolean(stream.exact&&!refiner.active&&stream.compositorRegionRevision===source.regionRevision);
}
