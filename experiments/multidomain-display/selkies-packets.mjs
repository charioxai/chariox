// MD-DISPLAY-03: callback failures become an awaited stream failure.
import {createInterface} from 'node:readline';
export function readPackets(stream,send,onFrame,onFailure){
 let ready=false,configured=false,failure;const dimensions=[];
 const fail=e=>{if(!failure){failure=e instanceof Error?e:Error(String(e));onFailure(failure)}};
 const lines=createInterface({input:stream.stdout});
 lines.on('line',line=>{
  if(failure)return;
  try{
   const record=JSON.parse(line);if(record.kind==='ready'){ready=true;return}if(record.kind!=='binary')return;
   const bytes=Buffer.from(record.data_base64,'base64');if(bytes[0]!==4)return;
   if(bytes.length<11)throw Error('MD-DISPLAY baseline truncated video packet');
   const key=bytes[1]===1,id=bytes.readUInt16BE(2),y=bytes.readUInt16BE(4),width=bytes.readUInt16BE(6),height=bytes.readUInt16BE(8),payload=bytes.subarray(10);
   if(y!==0)throw Error('MD-DISPLAY baseline unexpectedly striped');
   onFrame(bytes.length);dimensions.push([width,height,y]);let config;
   if(!configured&&key){
    let codec='avc1.640033';
    for(let i=0;i<payload.length-7;i++)if(payload[i]===0&&payload[i+1]===0&&payload[i+2]===1&&(payload[i+3]&31)===7){codec='avc1.'+payload.subarray(i+4,i+7).toString('hex').toUpperCase();break}
    config={codec,codedWidth:width,codedHeight:height,optimizeForLatency:true};configured=true;
   }
   if(configured)send({kind:'chunk',type:key?'key':'delta',timestamp:Math.round(performance.now()*1000),frame_id:id,config},payload);
  }catch(e){fail(e)}
 });
 stream.on('error',fail);stream.stdin.on('error',fail);
 return {dimensions,get ready(){if(failure)throw failure;return ready},check(){if(failure)throw failure},write(value){if(failure)return;try{stream.stdin.write(value)}catch(e){fail(e)}},close(){lines.close()}};
}
