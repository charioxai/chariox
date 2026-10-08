// MP-08/MP-10/MP-11: validate/decode every row before one synchronous canvas commit.
import {WorkerVideoDecoder} from './decoder-worker.mjs';
export function independentStripeCover(frame){
 if(frame.kind!=='stripes'||!Array.isArray(frame.stripes)||frame.stripes.length!==8)return false;
 const ordered=[...frame.stripes].sort((a,b)=>a.y-b.y),seen=new Set();let bottom=0;
 for(const row of ordered){
  if(!Number.isSafeInteger(row.row)||row.row<0||row.row>7||seen.has(row.row)||row.key!==true||row.reference_sequence!==null||row.y!==bottom||!Number.isSafeInteger(row.height)||row.height<1)return false;
  seen.add(row.row);bottom+=row.height;
 }
 return bottom===frame.height;
}
export class StripePresenter {
 constructor(){this.rows=new Map();}
 async decode(frame,decodeBytes){
  if(!Array.isArray(frame.stripes)||frame.stripes.length<1||frame.stripes.length>8)throw Error('MP-11: stripe count');
  const seen=new Set(),ordered=[...frame.stripes].sort((a,b)=>a.y-b.y);let bottom=0,total=0;
  for(const row of ordered){
   if(![row.row,row.y,row.height,row.sequence].every(Number.isSafeInteger)||row.row<0||row.row>7||row.y<bottom||row.height<1||row.y+row.height>frame.height||!['avc1.420033','vp8'].includes(row.codec)||typeof row.key!=='boolean'||seen.has(row.row)||row.sequence<1)throw Error('MP-11: stripe geometry');
   seen.add(row.row);bottom=row.y+row.height;
   total+=row.data?.byteLength??Infinity;if(total>4*1024*1024)throw Error('MP-11: stripe bytes');
   const previous=this.rows.get(row.row);
   if(row.key){if(row.reference_sequence!==null)throw Error('MP-11: stripe key reference');}
   else if(!previous||previous.y!==row.y||previous.height!==row.height||previous.sequence!==row.reference_sequence||row.sequence!==row.reference_sequence+1)throw Error('MP-10: stripe reference lost');
  }
  // A fresh canvas requires a complete independent row cover.
  if(this.rows.size===0&&(ordered[0].y!==0||bottom!==frame.height||ordered.some((r,i)=>!r.key||(i>0&&r.y!==ordered[i-1].y+ordered[i-1].height))))throw Error('MP-11: incomplete stripe bootstrap');
  const decoded=[];
  try{
   // MP-08/MP-10: rows have independent decoder chains. Wait for every job
   // even on failure so no transferred output survives an atomic abort.
   const jobs=await Promise.allSettled(ordered.map(async row=>{
    let state=this.rows.get(row.row);
    if(!state){state={decoder:new WorkerVideoDecoder()};this.rows.set(row.row,state)}
    const output=await state.decoder.decode({...row,width:frame.width,height:row.height},decodeBytes(row.data));
    decoded.push({row,output});
    if(output.displayWidth!==frame.width||output.displayHeight!==row.height)throw Error('MP-11: stripe decoded geometry');
   }));
   const failed=jobs.find(job=>job.status==='rejected');
   if(failed)throw failed.reason;
   return decoded;
  }catch(error){for(const {output} of decoded)output.close();this.close();throw error}
 }
 commit(decoded,context){for(const {row,output} of decoded){context.drawImage(output,0,row.y);Object.assign(this.rows.get(row.row),{sequence:row.sequence,y:row.y,height:row.height})}}
 close(){for(const row of this.rows.values())row.decoder.close();this.rows.clear();}
}
