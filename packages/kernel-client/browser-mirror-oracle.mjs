// MP-08/MP-10/MP-11 M0: independent, uncapped read-only CDP coverage oracle.
// No provider/page mutation or product observation cap. Keep the Cloud copy identical.
import assert from 'node:assert/strict'
export const oracleStyles = ['display','visibility','opacity','color','background-color','border-top-width','border-right-width','border-bottom-width','border-left-width','background-image']
export async function captureOracleSnapshot(send){
 const layout=await send('Page.getLayoutMetrics'),scale=layout.layoutViewport.clientWidth/layout.cssLayoutViewport.clientWidth
 assert(Number.isFinite(scale)&&scale>0,'MP-10: native CDP coordinate scale required')
 const snapshot=await send('DOMSnapshot.captureSnapshot',{computedStyles:oracleStyles,includePaintOrder:true,includeDOMRects:true})
 return {...snapshot,coordinateScale:scale}
}
const rare = (data, index) => { const at=data?.index?.indexOf(index)??-1; return at<0?null:data.value[at] }
const clipped = (b,w,h) => [Math.max(0,b[0]),Math.max(0,b[1]),Math.min(w,b[0]+b[2]),Math.min(h,b[1]+b[3])]
const visible = (b,w,h) => b[2]>0&&b[3]>0&&b[0]<w&&b[1]<h&&b[0]+b[2]>0&&b[1]+b[3]>0
export function oracleRecords(snapshot,root=0,width=1280,height=800) {
 const out=[],visited=new Map();let totalNodes=0,documents=0
 const intersect=(a,b)=>{const x=Math.max(a[0],b[0]),y=Math.max(a[1],b[1]);return [x,y,Math.max(0,Math.min(a[0]+a[2],b[0]+b[2])-x),Math.max(0,Math.min(a[1]+a[3],b[1]+b[3])-y)]}
 const visit=(snap,index,offset=[0,0],clip=[0,0,width,height])=>{
  assert(Array.isArray(snap.documents)&&snap.documents[index]&&Array.isArray(snap.strings),'MP-10: complete CDP DOMSnapshot required')
  const scale=snap.coordinateScale??1;assert(Number.isFinite(scale)&&scale>0,'MP-10: native CDP coordinate scale required')
  const seen=visited.get(snap)??new Set();assert(!seen.has(index),'MP-10: cyclic CDP documents');seen.add(index);visited.set(snap,seen)
  const d=snap.documents[index],n=d.nodes,l=d.layout,s=i=>snap.strings[i]??''
  assert(n&&l&&n.nodeName.length===n.nodeType.length,'MP-10: incomplete CDP nodes');totalNodes+=n.nodeName.length;documents++
  const layout=new Map(l.nodeIndex.map((node,i)=>[node,i]))
  const box=b=>[b[0]/scale-(d.scrollOffsetX??0)/scale+offset[0],b[1]/scale-(d.scrollOffsetY??0)/scale+offset[1],b[2]/scale,b[3]/scale]
  const stylesAt=i=>Object.fromEntries(oracleStyles.map((k,j)=>[k,s(l.styles[i]?.[j])]))
  for(let i=0;i<l.nodeIndex.length;i++){
   const ni=l.nodeIndex[i],name=s(n.nodeName[ni]).toLowerCase(),type=n.nodeType[ni],text=type===3?s(n.nodeValue[ni]):'',attrs={}
   for(let a=0;a<(n.attributes[ni]?.length??0);a+=2)attrs[s(n.attributes[ni][a])]=s(n.attributes[ni][a+1])
   const styles=stylesAt(i),b=intersect(box(l.bounds[i]),clip)
   const hidden=['hidden','collapse'].includes(styles.visibility)||styles.display==='none'||styles.opacity==='0'
   const painted=type===3?!!text.trim():['img','svg','input','textarea','select','button','canvas','video','iframe','object','embed'].includes(name)||styles['background-color']&&!['rgba(0, 0, 0, 0)','transparent'].includes(styles['background-color'])||styles['background-image']&&styles['background-image']!=='none'||['border-top-width','border-right-width','border-bottom-width','border-left-width'].some(k=>parseFloat(styles[k])>0)
   if(!hidden&&painted&&visible(b,width,height)&&!['script','style','noscript','template'].includes(name)){
    const fragments=[]
    if(type===3)for(let f=0;f<(d.textBoxes?.layoutIndex.length??0);f++)if(d.textBoxes.layoutIndex[f]===i){const q=intersect(box(d.textBoxes.bounds[f]),clip);if(q[2]>0&&q[3]>0)fragments.push(q)}
    // A multi-line text box can intersect a frame even when all its painted runs are clipped.
    if(type===3&&d.textBoxes?.layoutIndex.includes(i)&&!fragments.length)continue
    out.push({key:JSON.stringify([type,name,attrs.id??'',text]),type,name,text,box:b,fragments,color:styles.color})
   }
  }
  for(let ni=0;ni<n.nodeName.length;ni++){
   const child=rare(n.contentDocumentIndex,ni),name=s(n.nodeName[ni]).toLowerCase()
   if(child===null&&!['iframe','frame'].includes(name))continue
   const li=layout.get(ni);if(li===undefined)continue
   const styles=stylesAt(li);if(['hidden','collapse'].includes(styles.visibility)||styles.display==='none'||styles.opacity==='0')continue
   const b=box(l.bounds[li]),left=parseFloat(styles['border-left-width'])||0,top=parseFloat(styles['border-top-width'])||0,right=parseFloat(styles['border-right-width'])||0,bottom=parseFloat(styles['border-bottom-width'])||0
   const content=[b[0]+left,b[1]+top,Math.max(0,b[2]-left-right),Math.max(0,b[3]-top-bottom)],childClip=intersect(content,clip)
   if(!visible(childClip,width,height))continue
   if(child!==null)visit(snap,child,content.slice(0,2),childClip)
   else {const external=snap.externalFrames?.[s(d.frameId)+':'+ni];assert(external,'MP-10: visible out-of-process frame evidence missing');visit(external,0,content.slice(0,2),childClip)}
  }
 }
 visit(snapshot,root)
 return {records:out,totalNodes,documents,uncapped:true,width,height}
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

// MP-10: project only protected native pixels into the actual viewer footprint.
// This never reads client pixels to choose an exclusion or changes the viewer DOM.
export function expectedPanelRaster(native,clip,box,width,height){
 const data=new Uint8Array(width*height*4),sx=width/clip.width,sy=height/clip.height
 for(let y=0;y<height;y++)for(let x=0;x<width;x++){
  const u=Math.max(0,Math.min(native.width-1,((clip.x+(x+.5)/sx-box.x)/box.width)*native.width-.5)),v=Math.max(0,Math.min(native.height-1,((clip.y+(y+.5)/sy-box.y)/box.height)*native.height-.5))
  const x0=Math.floor(u),y0=Math.floor(v),x1=Math.min(x0+1,native.width-1),y1=Math.min(y0+1,native.height-1),dx=u-x0,dy=v-y0
  for(let c=0;c<4;c++){const a=native.pixels[(y0*native.width+x0)*4+c]*(1-dx)+native.pixels[(y0*native.width+x1)*4+c]*dx,b=native.pixels[(y1*native.width+x0)*4+c]*(1-dx)+native.pixels[(y1*native.width+x1)*4+c]*dx;data[(y*width+x)*4+c]=Math.round(a*(1-dy)+b*dy)}
 }
 return {width,height,data}
}
