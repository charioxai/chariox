import test from 'node:test'
import assert from 'node:assert/strict'
import {mirrorViewportStyle,mirrorScrollbarSize,mirrorInlineBaselineOffset} from './browser-mirror-layout.js'
import type {MirrorNode} from './browser-mirror-types.js'
const node:MirrorNode={id:'sup',parent:null,children:[],kind:'element',tag:'sup',box:{x:100,y:10,width:20,height:12},style:{'vertical-align':'super'}}
const current={x:100,y:12,width:20,height:12},style={display:'inline',position:'static',transform:'none'}
test('Retina font normalization preserves canonical visual and flow extent',()=>{
 for(const density of [1,1.5,2]){const css=mirrorViewportStyle(density);assert(css.includes(`zoom:${1/density}`));assert(css.includes(`transform:scale(${density})`));assert(css.includes(`margin-bottom:${800*(density-1)}px`))}
 for(const invalid of [0,NaN,Infinity,8])assert.throws(()=>mirrorViewportStyle(invalid))
})
test('native root gutter stays in CSS pixels despite iframe normalization',()=>{
 const root:MirrorNode={id:'html',parent:null,children:[],kind:'element',tag:'html',box:{x:0,y:0,width:1265,height:800},style:{width:'auto'}}
 assert.equal(mirrorScrollbarSize(root,1280),15);assert.equal(mirrorScrollbarSize({...root,style:{width:'70%'}},1280),null);assert.equal(mirrorScrollbarSize({...root,box:{...root.box!,width:1000}},1280),null)
})
test('only a dimension-matching native inline baseline can be projected',()=>{
 const records=new Map([[node.id,node]]);assert.equal(mirrorInlineBaselineOffset(node,records,current,style,[current]),-2)
 for(const [record,box,computed,fragments] of [
  [{...node,kind:'mask'},current,style,[current]], [{...node,tag:'button',style:{}},current,style,[current]],
  [node,{...current,y:20},style,[current]], [node,{...current,width:21},style,[current]], [node,{...current,x:102},style,[current]],
  [node,current,{...style,position:'relative'},[current]], [node,current,{...style,transform:'rotate(5deg)'},[current]], [node,current,style,[current,{...current,y:24}]],
 ] as const)assert.equal(mirrorInlineBaselineOffset(record as MirrorNode,records,box,computed,fragments),null)
 const parent:MirrorNode={id:'p',parent:null,children:['sup'],kind:'element',tag:'span',style:{transform:'rotate(5deg)'}}
 assert.equal(mirrorInlineBaselineOffset({...node,parent:'p'},new Map([['p',parent],['sup',node]]),current,style,[current]),null)
})

test('same-line bidi fragments retain the native baseline; wrapped fragments refuse',()=>{
 const records=new Map([[node.id,node]]),left={...current,width:4},middle={...current,x:104,width:12},right={...current,x:116,width:4}
 assert.equal(mirrorInlineBaselineOffset(node,records,current,style,[left,middle,right]),-2)
 for(const fragments of [[],[left,{...middle,y:13},right],[left,{...middle,height:10},right],[{...left,x:99},middle,right],[{...left,width:NaN}],Array(65).fill(current)])assert.equal(mirrorInlineBaselineOffset(node,records,current,style,fragments),null)
})
