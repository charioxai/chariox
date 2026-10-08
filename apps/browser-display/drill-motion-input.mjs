// MP-08/MP-10/MP-11: supplementary fixture typing while motion stays active.
// Input uses the admitted client path and echo uses presented pixels. These
// fixture results never count as real-app/public-site/hosted acceptance.
export async function measureMotionTyping(page, count = 20) {
 if(!Number.isSafeInteger(count)||count<1||count>20)throw Error('MP-10: typing probe count');
 return page.evaluate(async count => {
  const stamp=()=>performance.timeOrigin+performance.now(),samples=[];
  for(let expected=1;expected<=count;expected++){
   await mdStream.input({kind:'click',x:mdStream.presenter.canvas.width/mdStream.binding.device_scale_factor-170,y:28});
   await new Promise(resolve=>setTimeout(resolve,100));
   const started_ms=stamp();await mdStream.input({kind:'text',text:'a'});const input_ack_ms=stamp();
   // MP-08/MP-10: continuous redraw can replace the newest sample before its
   // rAF marker is filled. Keep the first visible echo after this input.
   const echo=()=>mdPresentations.find(sample=>sample.step===expected&&
    Number.isFinite(sample.presented_ms)&&sample.presented_ms>=started_ms&&sample.drawn_ms>=started_ms);
   let shown;
   while(!(shown=echo())){
    if(mdStream.error)throw mdStream.error;
    if(stamp()-started_ms>10000)throw Error('MP-10: typing during motion pixel acknowledgement timeout');
    await new Promise(resolve=>requestAnimationFrame(resolve));
   }
   const {drawn_ms,presented_ms}=shown;
   samples.push({started_ms,input_ack_ms,drawn_ms,presented_ms,latency_ms:presented_ms-started_ms});
  }
  return samples;
 },count);
}
