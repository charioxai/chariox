// MP-08/MP-10/MP-11: negative-only readiness; never authorize or emit pixels.
// The caller already checked scope/generation and renewed the admitted stream.
export function nativeCreditEmpty(stream,source,policy,epoch,changedAt,after,now=performance.now()){
 const producer=stream.producer,refiner=stream.refiner;
 if(!source?.attested||source.closed||source.policy!==policy||!source.valid?.()||!source.allowed(policy)||
    !stream.previous||stream.repair||!stream.acceptsCredit(after)||stream.creditEpoch!==epoch||
    producer?.source!==source||producer.failure||producer.frames.length||producer.active||producer.pending||
    !refiner||refiner.closed||refiner.failure)return false;
 const sample=source.sample();
 if(!sample?.raw||sample.serial!==stream.compositorSerial||sample.document_id!==stream.document_id||sample.tab_id!==stream.tab_id)return false;
 if(now-Math.max(source.changedAt,changedAt)<refiner.quietMs)return true;
 // Native exact bytes need no periodic recheck. Lossy CDP never enters here.
 const wanted=refiner.wanted;
 return Boolean(stream.exact&&refiner.latest&&!refiner.active&&wanted?.native===true&&
  wanted.source===source&&wanted.document===stream.document_id&&wanted.policy===policy&&wanted.epoch===epoch&&wanted.serial===sample.serial);
}
