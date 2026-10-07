// This pure function is embedded in the isolated observer, never page world.
// Exact private CSSOM/media/form state admits reuse; opacity, animation or an
// inaccessible stylesheet cannot be guessed from DOM mutation records alone.
export function mirrorCssFingerprint(roots,forms,custom,revision,variants,id){
 const sheets=new Set(),state=[];
 const sheet=(value,view)=>{
  if(sheets.has(value))return;sheets.add(value);
  state.push(value.disabled,value.media.mediaText,matchMedia.call(view,value.media.mediaText||'all').matches);
  const rules=items=>{for(const rule of items){state.push(rule.cssText);if(rule.conditionText&&rule.type===CSSRule.MEDIA_RULE)state.push(matchMedia.call(view,rule.conditionText).matches);if(rule.selectorText&&/:(state|autofill|playing|paused|buffering|stalled|muted|volume-locked|seeking|current|past|future)\b/.test(rule.selectorText))throw Error('dynamic CSS unavailable');if(rule.styleSheet)sheet(rule.styleSheet,view);if(rule.cssRules)rules(rule.cssRules)}};
  rules(value.cssRules);
 };
 for(const root of roots){const owner=root.ownerDocument??root,view=owner.defaultView;if(owner.getAnimations().length)throw Error('animated CSS');state.push(view.innerWidth,view.innerHeight,view.devicePixelRatio,view.scrollX,view.scrollY,owner.fonts.status,[...owner.fonts].map(font=>[font.family,font.style,font.weight,font.stretch,font.unicodeRange,font.status]));const nested=[...(root.styleSheets??[]),...(root.adoptedStyleSheets??[])];if(root.nodeType===11)for(const node of root.querySelectorAll('style,link'))if(node.sheet)nested.push(node.sheet);for(const value of nested)sheet(value,view)}
 const formState=forms.map(node=>[id(node),node.isConnected,node.checked,node.indeterminate,node.selected,node.value==='',node.validity?.valid]);
 const customState=custom.map(node=>[id(node),node.isConnected,Element.prototype.matches.call(node,':defined')]);
 const key=JSON.stringify([revision,variants,innerWidth,innerHeight,devicePixelRatio,scrollX,scrollY,location.hash,formState,customState,state]);
 if(key.length>2097152)throw Error('CSS cache memory budget');return key;
}
