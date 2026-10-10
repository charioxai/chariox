// Run in a browser attached to the already approved encrypted kernel transport.
// The coordinator supplies fixture-only callbacks; this module handles no keys.
import {attachNativeDisplay} from '../browser-display/native-presenter.mjs';

const check=(condition,message)=>{if(!condition)throw Error(message);};
async function decodedPixels(png,width,height) {
  const bitmap=await createImageBitmap(new Blob([png],{type:'image/png'}));
  try {
    check(bitmap.width===width&&bitmap.height===height,'independent source dimensions');
    const canvas=new OffscreenCanvas(width,height), context=canvas.getContext('2d');
    context.drawImage(bitmap,0,0);return context.getImageData(0,0,width,height).data;
  }finally{bitmap.close();}
}
function same(a,b){return a.length===b.length&&a.every((value,i)=>value===b[i]);}

export async function runNativeViewerDrill({canvas,transport,target,sourcePNG,change,retire}) {
  check(typeof sourcePNG==='function'&&typeof change==='function'&&typeof retire==='function','approved fixture callbacks required');
  check(transport.displayEventEncoding==='CXD1','authenticated CXD1 decoder required');
  const report={status:'FAIL',protocol:497,peer:106,checks:[]};
  let stream;
  const compare=async frame=>{
    check(frame&&frame.display_kind==='native','fresh native frame required');
    const expected=await decodedPixels(await sourcePNG(),canvas.width,canvas.height);
    const actual=canvas.getContext('2d').getImageData(0,0,canvas.width,canvas.height).data;
    check(same(actual,expected),'presented pixels differ from independent protected capture');
  };
  try {
    stream=await attachNativeDisplay(canvas,transport,target);
    const first=await stream.next();check(first?.kind==='png','exact bootstrap required');await compare(first);
    report.checks.push('encrypted exact bootstrap');
    await change();const damage=await stream.next();await compare(damage);
    check(damage.sequence>first.sequence,'changed source sequence');report.checks.push('native change reconstructs exact pixels');
    // Resubscription exercises lost-base repair without replaying an old credit.
    stream=await stream.repair();const repair=await stream.next();check(repair?.kind==='png','independent repair required');await compare(repair);
    report.checks.push('fresh subscription exact repair');
    const retired=await retire();check(retired?.revoked===true,'explicit native revoke acknowledgment required');
    let refused=false;try{await stream.next();}catch(error){
      refused=['user_domain_stale_epoch','user_domain_not_granted','user_domain_not_requested'].includes(error?.code);
    }
    check(refused,'retired pairing accepted another credit');report.checks.push('retired epoch refuses further capture');
    report.status='PASS';return report;
  }catch{throw Object.assign(Error("native viewer acceptance failed"),{receipt:report});}
  finally{await stream?.close().catch(()=>{});}
}
