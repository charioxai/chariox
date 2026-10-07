import test from 'node:test';
import assert from 'node:assert/strict';
import {mirrorCssFingerprint} from './kernel-browser-mirror-css.mjs';
function fixture(run){
 const names=['CSSRule','Element','matchMedia','innerWidth','innerHeight','devicePixelRatio','scrollX','scrollY','location'],old=new Map(names.map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]));
 const view={innerWidth:1280,innerHeight:800,devicePixelRatio:2,scrollX:0,scrollY:0};
 const font={family:'PublicFont',style:'normal',weight:'400',stretch:'normal',unicodeRange:'U+0000-00FF',status:'loaded'},fonts=new Set([font]);fonts.status='loaded';
 const rule={type:1,cssText:'p { color: blue; }',selectorText:'p'},sheet={disabled:false,media:{mediaText:''},cssRules:[rule]};
 const root={styleSheets:[sheet],adoptedStyleSheets:[],fonts,defaultView:view,getAnimations:()=>[]};
 Object.assign(globalThis,view,{CSSRule:{MEDIA_RULE:4},Element:{prototype:{matches(){return this.defined}}},matchMedia:()=>({matches:true}),location:{hash:''}});
 const read=()=>mirrorCssFingerprint([root],[],[],0,[],n=>n.key);
 try{run({root,font,rule,sheet,read})}finally{for(const [name,descriptor]of old)if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name]}
}
test('CSSOM changes invalidate the private cache without relying on DOM mutations',()=>fixture(({rule,read})=>{const before=read();assert.equal(read(),before);rule.cssText='p { color: red; }';assert.notEqual(read(),before)}));
test('font admission, media/disabled state and imports invalidate computed-style reuse',()=>fixture(({font,sheet,read})=>{let before=read();font.status='loading';assert.notEqual(read(),before);before=read();sheet.disabled=true;assert.notEqual(read(),before);before=read();sheet.cssRules.push({cssText:'@import "public.css";',styleSheet:{disabled:false,media:{mediaText:'all'},cssRules:[]}});assert.notEqual(read(),before)}));
test('an opaque sheet, running animation or unobservable dynamic selector disables CSS reuse',()=>fixture(({root,sheet,rule,read})=>{rule.selectorText=':state(private)';assert.throws(read,/dynamic CSS/);rule.selectorText='p';root.getAnimations=()=>[{}];assert.throws(read,/animated CSS/);root.getAnimations=()=>[];Object.defineProperty(sheet,'cssRules',{get(){throw Error('opaque stylesheet')}});assert.throws(read,/opaque stylesheet/)}));
