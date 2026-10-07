// Resolve the actual native fonts used by PUBLIC mirrored text. Only fixed
// system-font roots are read; page URLs/profile paths never become file reads.
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {realpath,open} from 'node:fs/promises';
import {constants} from 'node:fs';
import {createHash,randomUUID} from 'node:crypto';
import {deflateSync} from 'node:zlib';
const exec=promisify(execFile),align=n=>(n+3)&~3;
const fontCache=new Map();
export function mirrorFontKey(style){return JSON.stringify(['font-family','font-weight','font-style','font-stretch','font-variation-settings','font-feature-settings','font-size'].map(key=>style?.[key]??''));}
// W3C WOFF 1.0: preserve every sfnt table/checksum, compress tables separately.
// No metadata/private blocks or subsetting; browser font validation still runs.
function checksum(bytes){let sum=0;for(let i=0;i<bytes.length;i+=4){let word=0;for(let j=0;j<4;j++)word=(word<<8)|(bytes[i+j]??0);sum=(sum+(word>>>0))>>>0}return sum}
// Preserve outlines/advances. Native Linux metrics are rounded at the host's
// font scaler; a web font otherwise rounds again at the viewer's physical DPR.
// Freeze those public, size-specific ascent/descent metrics in standard sfnt
// tables, not in a protocol extension or client-side geometry adjustment.
export function normalizeFontMetrics(bytes,metrics){
 if(!metrics||!Number.isFinite(metrics.size)||metrics.size<=0||![metrics.ascent,metrics.descent].every(n=>Number.isFinite(n)&&n>=0&&n<=4*metrics.size))return bytes;
 const count=bytes.length>=12?bytes.readUInt16BE(4):0,tables=new Map();if(!count||count>256||12+16*count>bytes.length)return bytes;
 for(let i=0;i<count;i++){const at=12+16*i,start=bytes.readUInt32BE(at+8),length=bytes.readUInt32BE(at+12);if(start<12+16*count||start+length>bytes.length)return bytes;tables.set(bytes.toString('ascii',at,at+4),{at,start,length})}
 const head=tables.get('head'),hhea=tables.get('hhea'),os=tables.get('OS/2');
 if(!head||head.length<54||!hhea||hhea.length<36||tables.has('fvar')||tables.has('DSIG'))return bytes;
 const units=bytes.readUInt16BE(head.start+18),ascent=Math.round(metrics.ascent*units/metrics.size),descent=Math.round(metrics.descent*units/metrics.size);
 if(units<16||units>16384||ascent>32767||descent>32767)return bytes;
 const out=Buffer.from(bytes);out.writeUInt32BE(0,head.start+8);out.writeUInt16BE(out.readUInt16BE(head.start+16)|4096,head.start+16);
 out.writeInt16BE(ascent,hhea.start+4);out.writeInt16BE(-descent,hhea.start+6);
 if(os&&os.length>=78){out.writeInt16BE(ascent,os.start+68);out.writeInt16BE(-descent,os.start+70);out.writeUInt16BE(ascent,os.start+74);out.writeUInt16BE(descent,os.start+76)}
 for(const table of tables.values())out.writeUInt32BE(checksum(out.subarray(table.start,table.start+table.length)),table.at+4);
 out.writeUInt32BE((0xb1b0afba-checksum(out))>>>0,head.start+8);return out;
}
export function sfntToWoff(bytes,metrics){
 bytes=normalizeFontMetrics(bytes,metrics);
 if(!Buffer.isBuffer(bytes)||bytes.length<12||bytes.length>8*1024*1024||![0x00010000,0x4f54544f].includes(bytes.readUInt32BE(0)))throw Error('MP-11: unsupported system font');
 const count=bytes.readUInt16BE(4);if(!count||count>256||12+16*count>bytes.length)throw Error('MP-11: invalid font directory');
 const tables=[];let offset=44+20*count,sfnt=12+16*count,last=-1;
 for(let i=0;i<count;i++){
  const at=12+16*i,tag=bytes.readUInt32BE(at),start=bytes.readUInt32BE(at+8),length=bytes.readUInt32BE(at+12);
  if(tag<=last||start<12+16*count||start+length>bytes.length)throw Error('MP-11: invalid font table');last=tag;
  for(const table of tables)if(start<table.start+table.length&&start+length>table.start)throw Error('MP-11: overlapping font tables');
  const raw=bytes.subarray(start,start+length),compressed=deflateSync(raw),body=compressed.length<raw.length?compressed:raw;
  tables.push({tag,start,length,checksum:bytes.readUInt32BE(at+4),body,offset});offset+=align(body.length);sfnt+=align(length);
 }
 const out=Buffer.alloc(offset);out.write('wOFF');out.writeUInt32BE(bytes.readUInt32BE(0),4);out.writeUInt32BE(offset,8);out.writeUInt16BE(count,12);out.writeUInt32BE(sfnt,16);out.writeUInt16BE(1,20);
 for(let i=0;i<count;i++){const t=tables[i],at=44+20*i;out.writeUInt32BE(t.tag,at);out.writeUInt32BE(t.offset,at+4);out.writeUInt32BE(t.body.length,at+8);out.writeUInt32BE(t.length,at+12);out.writeUInt32BE(t.checksum,at+16);t.body.copy(out,t.offset)}
 if(out.toString('base64').length>700000)throw Error('MP-11: system font exceeds resource budget');return out;
}
export async function readSystemFont(postscript){
 if(process.platform!=='linux'||typeof postscript!=='string'||!/^[a-zA-Z0-9 _.-]{1,256}$/.test(postscript))return null;
 try{
  const {stdout}=await exec('fc-match',['--format=%{file}\n%{postscriptname}\n','--',':postscriptname='+postscript],{timeout:1000,maxBuffer:4096});
  const [file,name]=stdout.trim().split('\n');if(name!==postscript)return null;
  const resolved=await realpath(file);if(!['/usr/share/fonts/','/usr/local/share/fonts/'].some(root=>resolved.startsWith(root)))return null;
  const fileHandle=await open(resolved,constants.O_RDONLY|constants.O_NOFOLLOW);
  try{
   const info=await fileHandle.stat();if(!info.isFile()||info.size>8*1024*1024)return null;
   const key=JSON.stringify([resolved,info.dev,info.ino,info.size,info.mtimeMs]),cached=fontCache.get(key);if(cached)return cached;
   const body=await fileHandle.readFile();sfntToWoff(body);fontCache.set(key,body);while(fontCache.size>32)fontCache.delete(fontCache.keys().next().value);return body;
  }finally{await fileHandle.close()}
 }catch{return null}
}
export async function materializeMirrorLocalFonts(world,source,protectedValues,{readFont=readSystemFont,wireBudget=16*1024*1024,decodedBudget=64*1024*1024}={}){
 if(protectedValues.length)return {resources:[],fonts:[]};
 const {connection,sessionId,contextId}=world,objectGroup='mirror-fonts-'+randomUUID();
 try{
  const keysReply=await connection.send('Runtime.evaluate',{expression:'globalThis.__charioxMirror.fontKeys()',contextId,returnByValue:true},sessionId),keys=keysReply.result?.value;
  if(keysReply.exceptionDetails||!Array.isArray(keys)||keys.length>64||keys.some(k=>typeof k!=='string'||k.length>16384))throw Error('MP-11: public font set unavailable');
  if(!keys.length)return {resources:[],fonts:[]};
  const hosts=await connection.send('Runtime.evaluate',{expression:'globalThis.__charioxMirror.fontHosts()',contextId,objectGroup,returnByValue:false},sessionId);
  if(hosts.exceptionDetails||!hosts.result?.objectId)throw Error('MP-11: public font references unavailable');
  const props=await connection.send('Runtime.getProperties',{objectId:hosts.result.objectId,ownProperties:true},sessionId);
  const objects=props.result.filter(p=>/^(0|[1-9][0-9]*)$/.test(p.name)).sort((a,b)=>Number(a.name)-Number(b.name));
  if(objects.length!==keys.length||objects.some((p,i)=>Number(p.name)!==i||p.value?.subtype!=='node'||!p.value.objectId))throw Error('MP-11: invalid public font references');
  await connection.send('DOM.enable',{},sessionId);await connection.send('DOM.getDocument',{depth:0},sessionId);await connection.send('CSS.enable',{},sessionId);
  const admitted=new Map(),resources=new Map(),fonts=[],readFonts=new Map();let total=0,decoded=0;
  for(let i=0;i<objects.length;i++){
   const {nodeId}=await connection.send('DOM.requestNode',{objectId:objects[i].value.objectId},sessionId);
   const native=await connection.send('CSS.getPlatformFontsForNode',{nodeId},sessionId);
   // A web font already travels through the document/loader-bound resource path.
   // Mixed custom/system glyph runs need that path plus its native fallback;
   // don't substitute a guessed font when the native selection is ambiguous.
   if(!native.fonts?.length||native.fonts.some(f=>f.isCustomFont))continue;
   const measurement=native.fonts.length===1?await connection.send('Runtime.callFunctionOn',{objectId:objects[i].value.objectId,returnByValue:true,functionDeclaration:`function(){const s=this.ownerDocument.defaultView.getComputedStyle(this),c=this.ownerDocument.createElement('canvas').getContext('2d');c.font=s.font||[s.fontStyle,s.fontWeight,s.fontSize,s.fontFamily].join(' ');const m=c.measureText('M');return {size:parseFloat(s.fontSize),ascent:m.fontBoundingBoxAscent,descent:m.fontBoundingBoxDescent}}`},sessionId):null;
   const metrics=measurement?.exceptionDetails?null:measurement?.result?.value,aliases=[];
   for(const face of native.fonts){
    if(!readFonts.has(face.postScriptName))readFonts.set(face.postScriptName,await readFont(face.postScriptName));
    const raw=readFonts.get(face.postScriptName);if(!raw)continue;const body=sfntToWoff(raw,metrics);
    const resource_id=createHash('sha256').update(body).digest('hex'),family='cx-native-'+resource_id;
    if(!resources.has(resource_id)){const size=body.readUInt32BE(16);if(resources.size>=32||total+body.length>wireBudget||decoded+size>decodedBudget)continue;total+=body.length;decoded+=size;resources.set(resource_id,{resource_id,mime_type:'font/woff',data_base64:body.toString('base64')})}
    aliases.push(JSON.stringify(family));
    const style=JSON.parse(keys[i]);const weight=/^[0-9 ]+$/.test(style[1])?style[1]:'400',slant=['italic','oblique'].includes(style[2])?style[2]:'normal';
    if(!fonts.some(f=>f.family===family&&f.weight===weight&&f.style===slant))fonts.push({family,weight,style:slant,resource:resource_id});
   }
   if(aliases.length===native.fonts.length)admitted.set(keys[i],aliases.join(','));
  }
  for(const node of source.nodes){for(const style of [node.style,...Object.values(node.pseudo??{}).map(p=>p.style)])if(style){const family=admitted.get(mirrorFontKey(style));if(family)style['font-family']=family}}
  return {resources:[...resources.values()],fonts};
 }finally{await connection.send('Runtime.releaseObjectGroup',{objectGroup},sessionId).catch(()=>{})}
}
