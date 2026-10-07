// Native WOFFs carry fitted hhea metrics. CSS overrides apply those fractions
// after the viewer font scaler, which otherwise rounds them at its own DPR.
// Call only for verified kernel-native font/woff resources; origin fonts retain
// their ordinary font selection. Glyph data and advances remain untouched.
export async function nativeFontMetricOverrides(bytes:Uint8Array):Promise<{ascent:string;descent:string}|null>{
 try{
  const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength)
  if(bytes.length<84||view.getUint32(0)!==0x774f4646||view.getUint32(8)!==bytes.length)return null
  const count=view.getUint16(12),directory=44+20*count
  if(count<2||count>256||directory>bytes.length)return null
  const tables=new Map<string,{offset:number;length:number;original:number}>()
  for(let i=0;i<count;i++){
   const at=44+20*i,tag=String.fromCharCode(...bytes.subarray(at,at+4)),offset=view.getUint32(at+4),length=view.getUint32(at+8),original=view.getUint32(at+12)
   if(tables.has(tag)||offset<directory||offset%4||!length||length>original||offset+length>bytes.length)return null
   tables.set(tag,{offset,length,original})
  }
  const table=async(tag:string,size:number)=>{
   const info=tables.get(tag);if(!info||info.original!==size)throw Error('Native font metric table bound')
   const body=bytes.slice(info.offset,info.offset+info.length)
   if(info.length===size)return new DataView(body.buffer,body.byteOffset,body.byteLength)
   const reader=new Blob([body as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new DecompressionStream('deflate')).getReader(),out=new Uint8Array(size)
   let used=0
   try{while(true){const {value,done}=await reader.read();if(done)break;if(used+value.length>size)throw Error('Native font metric expansion bound');out.set(value,used);used+=value.length}}finally{await reader.cancel()}
   if(used!==size)throw Error('Native font metric length')
   return new DataView(out.buffer)
  }
  const head=await table('head',54),hhea=await table('hhea',36),units=head.getUint16(18),ascent=hhea.getInt16(4),descent=-hhea.getInt16(6)
  if(!(head.getUint16(16)&4096)||units<16||units>16384||ascent<=0||descent<0||ascent>units*4||descent>units*4)return null
  return {ascent:(ascent/units*100).toFixed(6)+'%',descent:(descent/units*100).toFixed(6)+'%'}
 }catch{return null}
}
