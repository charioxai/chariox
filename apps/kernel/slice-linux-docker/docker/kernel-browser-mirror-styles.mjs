// MP-08/MP-10/MP-11: trusted CSS initial values, never sampled from origin content.
// A short-lived blank target supplies this Chromium build's computed initial map.
export async function mirrorInitialStyles(browser, connection) {
  const {targetId}=await connection.send('Target.createTarget',{url:'about:blank',background:true});
  try {
    const {connection:page,sessionId}=await browser.resolvePageTarget(targetId);
    const reply=await page.send('Runtime.evaluate',{expression:`(()=>{const node=document.createElement('div');node.style.all='initial';document.body.append(node);const style=getComputedStyle(node);return Object.fromEntries([...style].map(key=>[key,style.getPropertyValue(key)]));})()`,returnByValue:true},sessionId);
    if(reply.exceptionDetails||!reply.result?.value||typeof reply.result.value!=='object')throw Error('MP-11: initial CSS unavailable');
    return reply.result.value;
  }finally {await connection.send('Target.closeTarget',{targetId});}
}
