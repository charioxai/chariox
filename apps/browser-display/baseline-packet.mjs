// MP-10: upstream Selkies 2.0.0 pixelflux stripe metadata only.
export function videoPacket(buffer){
 const bytes=new Uint8Array(buffer),v=new DataView(buffer);
 if(bytes[0]!==4)return null;
 if(bytes.length<13)throw Error('MP-10 truncated Selkies video header');
 const key=(bytes[1]&15)===1,id=v.getUint16(2),y=v.getUint16(4),width=v.getUint16(6),height=v.getUint16(8),payload=bytes.subarray(12);
 if(y!==0||width!==1920||height!==1080)throw Error('MP-10 unexpected Selkies geometry/stripe');
 let codec='avc1.64002A';
 for(let i=0;i<payload.length-7;i++)if(payload[i]===0&&payload[i+1]===0&&payload[i+2]===1&&(payload[i+3]&31)===7){codec='avc1.'+Array.from(payload.subarray(i+4,i+7),n=>n.toString(16).padStart(2,'0')).join('').toUpperCase();break;}
 return {key,id,width,height,payload,codec};
}
