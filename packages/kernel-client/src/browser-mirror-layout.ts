import type {MirrorNode} from './browser-mirror-types.js'

// The native host uses a physical-DPR1 font grid. Normalize the iframe's
// effective font scale while keeping its visual and flow extent at1280x800.
export function mirrorViewportStyle(density:number):string {
 if(!Number.isFinite(density)||density<.5||density>4)throw Error('MP-11: unsupported viewer density')
 return `zoom:${1/density};transform:scale(${density});transform-origin:top left;margin-right:${1280*(density-1)}px;margin-bottom:${800*(density-1)}px`
}
export function mirrorScrollbarSize(root:MirrorNode|undefined,width:number):number|null {
 if(root?.kind!=='element'||root.tag!=='html'||!root.box||root.box.x!==0||root.style?.width!=='auto')return null
 if(['margin-left','margin-right'].some(key=>parseFloat(root.style?.[key]??'0')!==0))return null
 const gutter=width-root.box.width
 return Number.isFinite(gutter)&&gutter>=0&&gutter<=32?gutter:null
}
// A used baseline is native geometry, not a portable CSS super/sub keyword.
// Adjust only an untransformed, static inline run whose dimensions and
// horizontal position already match. This never relaxes drift/input admission.
export function mirrorInlineBaselineOffset(record:MirrorNode,records:ReadonlyMap<string,MirrorNode>,current:{x:number;y:number;width:number;height:number},style:{display:string;position:string;transform:string},fragments:ReadonlyArray<{x:number;y:number;width:number;height:number}>):number|null {
 if(record.kind!=='element'||!record.box||!(['sup','sub'].includes(record.tag??'')||['super','sub'].includes(record.style?.['vertical-align']??''))||!fragments.length||fragments.length>64||style.display!=='inline'||style.position!=='static'||style.transform!=='none')return null
 for(let node:MirrorNode|undefined=record;node;node=records.get(node.parent??''))if(node.kind==='mask'||node.style?.transform&&node.style.transform!=='none'||node.style?.zoom&&!['1','normal'].includes(node.style.zoom))return null
 // Bidi isolates split a single line into adjacent boxes. Genuine wrapping
 // remains inadmissible; every fragment must share the same used baseline.
 if(fragments.some(rect=>![rect.x,rect.y,rect.width,rect.height].every(Number.isFinite)||rect.width<0||Math.abs(rect.y-current.y)>.015625||Math.abs(rect.height-current.height)>.015625||rect.x<current.x-.015625||rect.x+rect.width>current.x+current.width+.015625))return null
 const box=record.box,dy=box.y-current.y
 if(![current.x,current.y,current.width,current.height,dy].every(Number.isFinite)||Math.abs(box.x-current.x)>.5||Math.abs(box.width-current.width)>.5||Math.abs(box.height-current.height)>.5||Math.abs(dy)>3)return null
 return dy
}
