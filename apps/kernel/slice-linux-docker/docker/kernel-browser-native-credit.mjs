// MP-08/MP-10/MP-11: negative-only readiness; never authorize or emit pixels.
// The caller already checked scope/generation and renewed the admitted stream.
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
   now-Math.max(source.changedAt,changedAt)<quiet&&stream.canPatchNative?.(sample)===false;
 }
 if(sample.serial!==stream.compositorSerial)return false;
 if(now-Math.max(source.changedAt,changedAt)<quiet)return true;
 // MP-08/MP-10/MP-11: complete exact bytes remain exact after every
 // admitted contiguous native tile patch. No lossy-source equality inference.
 return Boolean(stream.exact&&!refiner.active&&stream.compositorRegionRevision===source.regionRevision);
}
