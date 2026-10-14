// MP-08/MP-10/MP-11 M0: independent, uncapped read-only CDP coverage oracle.
// No provider/page mutation or product observation cap. Keep the Cloud copy identical.
import assert from 'node:assert/strict'
export const oracleStyles = ['display','visibility','opacity','color','background-color','border-top-width','border-right-width','border-bottom-width','border-left-width']
export const captureOracleSnapshot = send => send('DOMSnapshot.captureSnapshot', {computedStyles:oracleStyles,includePaintOrder:true,includeDOMRects:true})
const rare = (data, index) => { const at=data?.index?.indexOf(index)??-1; return at<0?null:data.value[at] }
const clipped = (b,w,h) => [Math.max(0,b[0]),Math.max(0,b[1]),Math.min(w,b[0]+b[2]),Math.min(h,b[1]+b[3])]
const visible = (b,w,h) => b[2]>0&&b[3]>0&&b[0]<w&&b[1]<h&&b[0]+b[2]>0&&b[1]+b[3]>0
export function oracleRecords(snapshot,root=0,width=1280,height=800) {
 assert(Array.isArray(snapshot.documents)&&snapshot.documents[root]&&Array.isArray(snapshot.strings),'MP-10: complete CDP DOMSnapshot required')
 const out=[],visited=new Set(),s=i=>snapshot.strings[i]??''
 const visit=(index,offset=[0,0])=>{
  assert(!visited.has(index),'MP-10: cyclic CDP documents');visited.add(index)
  const d=snapshot.documents[index],n=d.nodes,l=d.layout;assert(n&&l&&n.nodeName.length===n.nodeType.length,'MP-10: incomplete CDP nodes')
  const layout=new Map(l.nodeIndex.map((node,i)=>[node,i]))
  for(let i=0;i<l.nodeIndex.length;i++){
   const ni=l.nodeIndex[i],name=s(n.nodeName[ni]).toLowerCase(),type=n.nodeType[ni],text=type===3?s(n.nodeValue[ni]):'',attrs={}
   for(let a=0;a<(n.attributes[ni]?.length??0);a+=2)attrs[s(n.attributes[ni][a])]=s(n.attributes[ni][a+1])
   const styles=Object.fromEntries(oracleStyles.map((k,j)=>[k,s(l.styles[i]?.[j])]))
   const b=l.bounds[i],box=[b[0]-(d.scrollOffsetX??0)+offset[0],b[1]-(d.scrollOffsetY??0)+offset[1],b[2],b[3]]
   const hidden=['hidden','collapse'].includes(styles.visibility)||styles.display==='none'||styles.opacity==='0'
   const painted=type===3?!!text.trim():['img','svg','input','textarea','select','button','canvas','video','iframe','object','embed'].includes(name)||styles['background-color']&&!['rgba(0, 0, 0, 0)','transparent'].includes(styles['background-color'])||['border-top-width','border-right-width','border-bottom-width','border-left-width'].some(k=>parseFloat(styles[k])>0)
   if(!hidden&&painted&&visible(box,width,height)&&!['script','style','noscript','template'].includes(name)){
    const fragments=[]
    if(type===3)for(let f=0;f<(d.textBoxes?.layoutIndex.length??0);f++)if(d.textBoxes.layoutIndex[f]===i){const q=d.textBoxes.bounds[f];fragments.push([q[0]-(d.scrollOffsetX??0)+offset[0],q[1]-(d.scrollOffsetY??0)+offset[1],q[2],q[3]])}
    out.push({key:JSON.stringify([type,name,attrs.id??'',text]),type,name,text,box,fragments,color:styles.color})
   }
  }
  for(let ni=0;ni<n.nodeName.length;ni++){
   const child=rare(n.contentDocumentIndex,ni);if(child===null)continue
   const li=layout.get(ni);if(li===undefined)continue
   const b=l.bounds[li];if(visible([b[0]-(d.scrollOffsetX??0)+offset[0],b[1]-(d.scrollOffsetY??0)+offset[1],b[2],b[3]],width,height))visit(child,[offset[0]+b[0]-(d.scrollOffsetX??0),offset[1]+b[1]-(d.scrollOffsetY??0)])
  }
 }
 visit(root)
 return {records:out,totalNodes:[...visited].reduce((sum,i)=>sum+snapshot.documents[i].nodes.nodeName.length,0),documents:visited.size,uncapped:true,width,height}
}
// Exact rectangle union: overlapping missing paint cannot be counted repeatedly.
export function unionArea(boxes,width=1280,height=800){
 const rects=boxes.map(b=>clipped(b,width,height)).filter(b=>b[2]>b[0]&&b[3]>b[1]),xs=[...new Set(rects.flatMap(b=>[b[0],b[2]]))].sort((a,b)=>a-b);let area=0
 for(let i=1;i<xs.length;i++){
  const ys=rects.filter(b=>b[0]<xs[i]&&b[2]>xs[i-1]).map(b=>[b[1],b[3]]).sort((a,b)=>a[0]-b[0]);let height=0,end=-Infinity
  for(const [a,b]of ys){height+=Math.max(0,b-Math.max(a,end));end=Math.max(end,b)}
  area+=(xs[i]-xs[i-1])*height
 }
 return area
}
const distance=(a,b)=>Math.max(...a.map((x,i)=>Math.abs(x-b[i])))
export function mirrorCoverageMetrics(source,viewer){
 assert(source.uncapped&&viewer.uncapped,'MP-10: capped observations cannot establish coverage')
 assert.equal(source.width,viewer.width);assert.equal(source.height,viewer.height)
 const pool=new Map();for(const r of viewer.records){const list=pool.get(r.key)??[];list.push(r);pool.set(r.key,list)}
 const missing=[],drift=[];let texts=0,textHits=0,matched=0,max=0,colorMismatches=0,fragmentMismatches=0
 for(const r of source.records){
  if(r.type===3)texts++
  const list=pool.get(r.key)??[];let best=-1,delta=Infinity
  for(let i=0;i<list.length;i++){const d=distance(r.box,list[i].box);if(d<delta){delta=d;best=i}}
  if(best<0){missing.push(r);continue}
  const [v]=list.splice(best,1);matched++;if(r.type===3)textHits++
  if(r.type===3){if(r.color!==v.color)colorMismatches++;if(r.fragments.length!==v.fragments.length)fragmentMismatches++;else for(let i=0;i<r.fragments.length;i++)delta=Math.max(delta,distance(r.fragments[i],v.fragments[i]))}
  max=Math.max(max,delta);if(delta>.5)drift.push({name:r.name,text:r.text.slice(0,80),source:r.box,viewer:v.box,delta})
 }
 const painted=unionArea(source.records.map(r=>r.box),source.width,source.height),lost=unionArea(missing.map(r=>r.box),source.width,source.height)
 return {C_area:painted?1-lost/painted:null,C_text:texts?textHits/texts:null,area:{painted,missing:lost,denominator:'union of visible native paint boxes; subtract union of unmatched native boxes'},text:{total:texts,mirrored:textHits},drift:{maxCssPx:max,overHalfPixel:drift.length,colorMismatches,fragmentMismatches,missingPaint:missing.length,examples:drift.slice(0,20)},sourceNodes:source.totalNodes,viewerNodes:viewer.totalNodes,sourceDocuments:source.documents,matched,missing:missing.slice(0,20).map(r=>({name:r.name,text:r.text.slice(0,80),box:r.box})),uncapped:true}
}
export function mirrorMetricFailures(metrics){
 const out=[]
 if(metrics.C_area!==1)out.push('C_area below 1')
 if(metrics.C_text!==1)out.push('C_text below 1')
 if(metrics.drift.maxCssPx>.5||metrics.drift.colorMismatches||metrics.drift.fragmentMismatches||metrics.drift.missingPaint)out.push('native/mirror geometry, text fragments or colors differ')
 if(!Number.isFinite(metrics.raster?.outside_edge_mismatch_fraction)||metrics.raster.outside_edge_mismatch_fraction>.001)out.push('raster outside-edge mismatch exceeds 0.10%')
 return out
}
