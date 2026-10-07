// Embedded into the isolated observer. Scan all source text before viewport
// projection, retaining only enough overlap to recognize split-node echoes.
export function protectedTextScanner(variants,marked){
 const values=variants.filter(value=>typeof value==='string'&&value.length>0);
 const longest=values.reduce((max,value)=>Math.max(max,value.length),0);
 if(longest>2097152)throw Error('mirror protection dictionary budget');
 let tail='',tailNodes=[];
 return node=>{
  if(!longest)return;
  for(let at=0;at<node.data.length;at+=16384){
   const part=node.data.slice(at,at+16384),joined=tail+part,parts=[...tailNodes,{node,start:tail.length,length:part.length}];
   for(const value of values)for(let found=joined.indexOf(value);found>=0;found=joined.indexOf(value,found+1))for(const item of parts)if(item.start<found+value.length&&item.start+item.length>found)marked.add(item.node);
   const start=Math.max(0,joined.length-longest+1);
   tail=joined.slice(start);tailNodes=parts.filter(item=>item.start+item.length>start).map(item=>({node:item.node,start:Math.max(0,item.start-start),length:item.length-Math.max(0,start-item.start)}));
  }
 };
}
