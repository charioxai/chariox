import {protectedTextScanner} from './kernel-browser-mirror-protected-text.mjs';
import {mirrorCssFingerprint} from './kernel-browser-mirror-css.mjs';
import {mirrorFontKey} from './kernel-browser-mirror-local-fonts.mjs';
// MP-08/MP-10/MP-11: install only in the controller's #607 isolated world.
// Raw mutation records and page code never cross the mirror boundary.
export const mirrorObserverExpression = initial => `(${installMirrorObserver.toString()})(${JSON.stringify(initial)},${mirrorCssFingerprint.toString()},${protectedTextScanner.toString()},${mirrorFontKey.toString()})`;
function installMirrorObserver(initialStyles = {},inspectCss,makeTextScanner,fontKey) {
  if (globalThis.__charioxMirror) return true;
  const nativeCustom=new WeakMap(),opaqueFlow=new WeakMap();
  const ids = new WeakMap(),eventRoots=new WeakSet();let observed = new WeakSet(),cssCache=new WeakMap(),cssFingerprint=null;
  let lastCssRoots=new Set(),lastCssForms=[],lastCssCustom=[],fontRepresentatives=[];
  let serial = 0, live = new Map(), revision = 0, protectedVariants=[], maskedNodes=new WeakSet();
  const observer = new MutationObserver(() => { revision++; });
  const watch = root => {
    if (observed.has(root)) return;
    if(!eventRoots.has(root)){eventRoots.add(root);for(const event of ['focusin','focusout','pointerover','pointerout','pointerdown','pointerup','input','change','scroll','keydown','keyup','selectionchange','beforetoggle','toggle','fullscreenchange','load','error','resize'])root.addEventListener(event,()=>{revision++},true)}
    observed.add(root); observer.observe(root, { subtree:true, childList:true, attributes:true, characterData:true });
  };
  const id = node => { if (!ids.has(node)) ids.set(node, `n${++serial}`); return ids.get(node); };
  const tags = new Set('html head body div span p a article section main header footer nav aside h1 h2 h3 h4 h5 h6 ul ol li dl dt dd pre code blockquote b strong i em u s small sub sup br wbr hr table thead tbody tfoot tr th td caption colgroup col input textarea button select option optgroup label fieldset legend form details summary dialog img figure figcaption picture source slot'.split(' '));
  const attributes = new Set('title alt role aria-label aria-hidden aria-expanded aria-checked aria-selected aria-disabled slot dir lang colspan rowspan span type placeholder disabled readonly multiple size rows cols wrap open start reversed value checked selected'.split(' '));
  const active = new Set('script style link meta base noscript template'.split(' '));
  const media = new Set('canvas video audio svg object embed applet'.split(' '));
  const forbiddenCss = /url\s*\(|image-set\s*\(|(?:-webkit-)?image\s*\(|expression\s*\(|@|\\|[<>]|[\u0000-\u0008]/i;
  const box = node => {
    const r = node.nodeType === 1 ? node.getBoundingClientRect() : (() => { const range=document.createRange(); range.selectNodeContents(node); return range.getBoundingClientRect(); })();
    return {x:r.x,y:r.y,width:r.width,height:r.height};
  };
  const snapshots = new Map();
  const read = (variants = [], opaqueRegions = [], subscription = null, reset = false) => {
    if(observer.takeRecords().length)revision++;
    observer.disconnect();observed=new WeakSet();
    protectedVariants=variants;
    const records = [], resources = [], fonts = [], nextLive = new Map(), marked = new WeakSet();
    const serializedRecords=new Map(),accountedStyles=new Set(),viewportAncestors=new WeakSet(),styleRoots=new Set([document]),formNodes=[],customNodes=[];
    let wireSize=0;const done=record=>{const serialized=JSON.stringify(record);serializedRecords.set(record.id,serialized);const {style,...rest}=record;wireSize+=JSON.stringify(rest).length;if(style){const key=JSON.stringify(style);if(!accountedStyles.has(key)){accountedStyles.add(key);wireSize+=key.length}}if(wireSize>32*1024*1024)throw new Error('mirror snapshot bounds');return record.id;};
    let textSize=0,visited=0;
    const scanText=makeTextScanner(variants,marked);
    const tainted = value => typeof value==='string' && variants.some(secret => value.includes(secret));
    // Match unsplit text BEFORE truncation, including split-node/shadow/frame echoes.
    const scan = (node,depth=0) => {
      if (depth>128 || ++visited>400000) throw new Error('mirror bounds');
      // Viewport projection is independent of protection: scan the ENTIRE source
      // first, including deferred/shadow/frame text. Retain any ancestor of a
      // visible descendant so fixed/sticky or overflowing children cannot vanish
      // merely because their flow parent's own rectangle is offscreen.
      if(node.nodeType===1){if(['input','textarea','select','option'].includes(node.localName))formNodes.push(node);if(node.localName.includes('-'))customNodes.push(node);const r=node.getBoundingClientRect();if(r.width>0&&r.height>0&&r.x<innerWidth+800&&r.x+r.width>-800&&r.y<innerHeight+800&&r.y+r.height>-800){for(let ancestor=node;ancestor;ancestor=ancestor.parentElement??ancestor.getRootNode()?.host??ancestor.ownerDocument?.defaultView?.frameElement)viewportAncestors.add(ancestor)}}
      if(node.nodeType===3)scanText(node);
      for (const child of node.childNodes) scan(child,depth+1);
      if (node.shadowRoot) {styleRoots.add(node.shadowRoot);scan(node.shadowRoot,depth+1);}
      if (node.localName==='iframe') {let nested;try{nested=node.contentDocument;}catch{}if(nested){styleRoots.add(nested);scan(nested,depth+1);}}
    };
    scan(document.documentElement);
    const secret = node => node.nodeType===1 && (node.matches('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]') || /password|one-time-code|cc-/i.test(node.autocomplete??'') || tainted(node.value) || [...node.attributes].some(a=>tainted(a.value)));
    // MP-08/MP-10/MP-11: exact same-read CSS sharing for plain leaf paragraphs.
    // Same parent, tag, raw attributes, box size and complete selector match set
    // imply the same author cascade + inheritance. Uninspectable/grouped/nested/
    // pseudo CSS, shadows, frames and animations disable it. Nothing persists
    // across reads, so property-only or stylesheet mutations cannot stale it.
    let selectors=null;
    if(!variants.length&&!document.getAnimations().length)try {
      selectors=[];
      for(const sheet of [...document.styleSheets,...document.adoptedStyleSheets])for(const rule of sheet.cssRules) {
        if(rule.type===CSSRule.FONT_FACE_RULE)continue;
        if(rule.type!==CSSRule.STYLE_RULE||rule.cssRules?.length||rule.selectorText.includes('::')||selectors.length>=64)throw new Error('mirror CSS sharing unavailable');
        selectors.push(rule.selectorText);
      }
    }catch{selectors=null;}
    // Persistent CSS reuse is admitted only when EVERY contributing sheet is
    // inspectable and its exact CSSOM/media state, DOM revision, source viewport,
    // interaction and form pseudo-state match. Never reuse across animation,
    // opaque CSS, policy changes or a changed stylesheet. Geometry/protection
    // still resample the full source on every credit.
    let reusableCss=true;lastCssRoots=styleRoots;lastCssForms=formNodes;lastCssCustom=customNodes;
    try{const fingerprint=inspectCss(styleRoots,formNodes,customNodes,revision,variants,id);if(fingerprint!==cssFingerprint){cssCache=new WeakMap();cssFingerprint=fingerprint}}catch{reusableCss=false;cssCache=new WeakMap();cssFingerprint=null}
    const sharedStyles=new Map();
    // A deferred auto-height block still carries its anonymous flow and the
    // margins which collapse through transparent first/last descendant blocks.
    // Read CSS geometry only; no deferred text/content enters the placeholder.
    const flowMargin=(element,edge)=>{
      let positive=0,negative=0;
      for(let e=element,depth=0;e&&depth<128;depth++){
        const s=getComputedStyle(e),margin=parseFloat(s.getPropertyValue('margin-'+edge))||0;positive=Math.max(positive,margin);negative=Math.min(negative,margin);
        const child=edge==='top'?e.firstElementChild:e.lastElementChild;
        if(!child||!['block','list-item'].includes(s.display)||!['static','relative'].includes(s.position)||s.float!=='none'||s.overflowY!=='visible'||parseFloat(s.getPropertyValue('padding-'+edge))||parseFloat(s.getPropertyValue('border-'+edge+'-width'))||edge==='bottom'&&e.computedStyleMap?.().get('height')?.toString()!=='auto')break;
        const c=getComputedStyle(child);if(!['block','list-item','flow-root'].includes(c.display)||!['static','relative'].includes(c.position)||c.float!=='none')break;
        const a=e.getBoundingClientRect(),b=child.getBoundingClientRect();if(Math.abs(a[edge]-b[edge])>.1)break;e=child;
      }
      return positive+negative;
    };
    const safeStyle = (element,pseudo=null,resourcesAllowed=true,bounds=null) => {
      const computed=getComputedStyle(element,pseudo),cached=reusableCss?cssCache.get(element)?.get(pseudo):null;
      const out=cached?{...cached}:{all:'initial'};
      let sharedKey=null;
      if(selectors&&!pseudo&&bounds&&element.ownerDocument===document&&element.getRootNode()===document&&element.localName==='p'&&element.attributes.length<=16&&[...element.attributes].every(a=>a.value.length<=2048)&&element.childNodes.length===1&&element.firstChild.nodeType===3&&computed.backgroundImage==='none') {
        sharedKey=JSON.stringify([id(element.parentNode),element.localName,[...element.attributes].map(a=>[a.name,a.value]),bounds.width,bounds.height,selectors.map(selector=>element.matches(selector))]);
        if(sharedStyles.has(sharedKey))return {...sharedStyles.get(sharedKey)};
      }
      if(!cached)for (const property of computed) {
        if(property.startsWith('--') || property.startsWith('animation') || property.startsWith('transition') || ['content','cursor','inline-size','block-size','min-inline-size','min-block-size','max-inline-size','max-block-size'].includes(property)) continue;
        const value=computed.getPropertyValue(property);
        if(value===initialStyles[property] && !['direction','unicode-bidi','color','background-color','border-top-color','border-right-color','border-bottom-color','border-left-color'].includes(property))continue;
        if(value.length<=2048 && !forbiddenCss.test(value) && !tainted(value)) out[property]=value;
      }
      // CSSOM resolved height turns auto into pixels, changing margin collapse,
      // flex sizing and intrinsic flow. Typed OM retains the computed sizing
      // contract. Native tiles keep their used dimensions for their blank text.
      if(!pseudo&&element instanceof HTMLElement&&!['img','iframe','video','canvas','object','embed'].includes(element.localName)&&!(['button','input','textarea','select'].includes(element.localName)&&computed.appearance!=='none')&&element.computedStyleMap){
        const typed=element.computedStyleMap();for(const property of ['width','height','min-width','min-height','max-width','max-height']){
          const value=typed.get(property)?.toString();if(value&&value.length<=2048&&!forbiddenCss.test(value)&&!tainted(value))out[property]=value;
        }
      }
      if(!pseudo&&computed.transform==='none'&&computed.writingMode==='horizontal-tb'&&['auto','scroll'].includes(computed.overflowY)&&element.offsetWidth-element.clientWidth-parseFloat(computed.borderLeftWidth)-parseFloat(computed.borderRightWidth)>1)out['scrollbar-gutter']='stable';
      if(reusableCss&&!cached){let entries=cssCache.get(element);if(!entries){entries=new Map();cssCache.set(element,entries)}entries.set(pseudo,{...out})}
      // Inline URLs are never shipped. Computed image URLs become kernel resource refs.
      if (!pseudo && !variants.length && resourcesAllowed) {
        const value=computed.backgroundImage, match=/^url\("([^"\n]+)"\)$/.exec(value);
        if(match && !tainted(match[1])) { const key=`r${resources.length}`; resources.push({key,url:match[1],kind:'image'}); out['background-image']=`resource:${key}`; }
      }
      if(sharedKey)sharedStyles.set(sharedKey,out);
      return out;
    };
    const visit = (node,parent=null,depth=0) => {
      if(depth>128 || records.length>=100000) throw new Error('mirror node bounds');
      if(![1,3,9,11].includes(node.nodeType)) return null;
      if(node.nodeType===1 && active.has(node.localName)) return null;
      const record={id:id(node),parent,children:[],kind:'element'};
      nextLive.set(record.id,node); records.push(record);
      if(node.nodeType===3) {
        record.kind=marked.has(node)?'mask':'text'; record.text=marked.has(node)?'':node.data;record.box=box(node);if(record.kind==='mask'){record.box=box(node);record.tag='div';record.style={display:'inline-block',width:`${record.box.width}px`,height:`${record.box.height}px`,background:'black'};}
        textSize+=record.text.length; if(textSize>2097152) throw new Error('mirror text bounds');
        return done(record);
      }
      if(node.nodeType===9 || node.nodeType===11) { record.kind=node.nodeType===9?'document':'shadow'; watch(node); }
      else {
        const tag=node.localName;
        record.tag=tags.has(tag)?tag:'div'; record.box=box(node);
        const overlaps=opaqueRegions.some(r=>record.box.width>0 && record.box.height>0 && record.box.x<r[0]+r[2] && record.box.x+record.box.width>r[0] && record.box.y<r[1]+r[3] && record.box.y+record.box.height>r[1]);
        if(secret(node) || overlaps && !['html','body'].includes(tag)) {
          record.kind='mask'; record.tag=tags.has(tag)?tag:'div'; record.style={...safeStyle(node,null,false),width:`${record.box.width}px`,height:`${record.box.height}px`,background:'black',color:'transparent','border-color':'black'};
          return done(record);
        }
        record.style=safeStyle(node,null,true,record.box);
        // CSS Display/Position: display:none suppresses shadow-inclusive descendants,
        // including top-layer boxes. Full protection scan precedes this omission.
        if(record.style.display==='none')return done(record);
        // Bound the live DOM working set by the source viewport, not page size.
        // Flow placeholders retain resolved CSS dimensions. Moving the source
        // viewport hydrates newly visible descendants and evicts old offscreen
        // subtrees through the ordinary authenticated delta/removal path.
        // Pseudo paint/effects may extend beyond element bounds: do not defer it.
        if(!viewportAncestors.has(node)&&record.box.width>0&&record.box.height>0&&['div','section','article','main','aside','p','pre','li','ul','ol'].includes(tag)&&['block','list-item'].includes(record.style.display)&&['static','relative'].includes(record.style.position??'static')&&!node.shadowRoot&&getComputedStyle(node).filter==='none'&&getComputedStyle(node).boxShadow==='none'&&getComputedStyle(node).textShadow==='none'&&['::before','::after'].every(p=>['none','normal','""'].includes(getComputedStyle(node,p).content))) {
          record.kind='tile';record.reason='viewport_deferred';
          if(record.style.height==='auto'){
            const computed=getComputedStyle(node),children=[...node.children],first=children[0],last=children.at(-1),top=first?flowMargin(first,'top'):0,bottom=last?flowMargin(last,'bottom'):0;
            const paddingTop=parseFloat(computed.paddingTop)||0,paddingBottom=parseFloat(computed.paddingBottom)||0,borderTop=parseFloat(computed.borderTopWidth)||0,borderBottom=parseFloat(computed.borderBottomWidth)||0;
            const height=record.box.height-paddingTop-paddingBottom-borderTop-borderBottom;
            const internalTop=first?Math.max(0,first.getBoundingClientRect().top-record.box.y-paddingTop-borderTop):0,internalBottom=last?Math.max(0,record.box.y+record.box.height-paddingBottom-borderBottom-last.getBoundingClientRect().bottom):0;
            record.pseudo={'::before':{text:'',style:{all:'initial',display:'block',height:Math.max(0,height-Math.min(internalTop,top)-Math.min(internalBottom,bottom))+'px','margin-top':top+'px','margin-bottom':bottom+'px'}}};
          }
          return done(record);
        }
        record.attributes={};
        for(const attr of node.attributes) if((attributes.has(attr.name)||tag==='slot'&&attr.name==='name') && attr.value.length<=2048 && !tainted(attr.value)) record.attributes[attr.name]=attr.value;
        // MP-08/MP-11: preserve inert editing semantics, never arbitrary values.
        if(node.hasAttribute('contenteditable'))record.attributes.contenteditable=['true','false','plaintext-only'].includes(node.contentEditable)?node.contentEditable:(node.isContentEditable?'true':'false');
        // No name/id/data-* attributes, URLs, event handlers, provider/page secrets.
        if(tag==='input' && !['text','search','email','url','number','tel','checkbox','radio','range','button','submit','reset','date','time','color','hidden'].includes(record.attributes.type??'text')) record.attributes.type='text';
        if(['input','textarea','select'].includes(tag)&&typeof node.value==='string'&&node.value.length>16384)throw new Error('mirror form bounds');
        if(tag==='input' || tag==='textarea' || tag==='select') record.form={value:(node.value??'').slice(0,16384),checked:!!node.checked,selected_index:node.selectedIndex??-1,selection_start:node.selectionStart??null,selection_end:node.selectionEnd??null};
        record.scroll={x:node.scrollLeft,y:node.scrollTop};
        if(['button','input','textarea','select'].includes(tag) && getComputedStyle(node).appearance!=='none'){record.kind='tile';record.reason='native_control';return done(record);}
        // URL-backed masks/border images cannot become a flat coloured shape
        // when their executable paint source is stripped. Use protected native
        // pixels for this region, retaining the page's DOM elsewhere.
        const opaquePaint=style=>['mask-image','-webkit-mask-image','border-image-source'].some(property=>style.getPropertyValue(property)!=='none'&&forbiddenCss.test(style.getPropertyValue(property)));
        if(opaquePaint(getComputedStyle(node))||['::before','::after'].some(p=>{const s=getComputedStyle(node,p);return !['none','normal'].includes(s.content)&&(opaquePaint(s)||s.backgroundImage!=='none')})){
          record.kind='tile';record.reason='unsupported_paint';
          // Retain sanitized public descendants for native flex/inline baseline
          // layout. Protected children still terminate at opaque masks. Paint
          // is composited separately; replacing the host with IMG changes flow.
        }
        if(media.has(tag) || tag.includes('-') && !node.shadowRoot&&nativeCustom.get(node)!=='light') {record.kind='tile';record.tag='img';record.reason=tag.includes('-')?'opaque_shadow':'opaque_media';const flow=opaqueFlow.get(node);if(flow)record.style={...record.style,...flow};return done(record);}
        if(tag==='iframe') {
          try {
            const nested=node.contentDocument;
            if(!nested?.documentElement) throw new Error('cross origin');
            // Use a separate inert nested document, never its original src/srcdoc.
            record.kind='frame';record.tag='iframe';record.children.push(visit(nested,record.id,depth+1));return done(record);
          } catch {record.kind='tile';record.tag='img';record.reason='cross_origin_frame';return done(record);}
        }
        if(tag==='img') {
          if(variants.length) {record.kind='mask';record.tag='div';return done(record);}
          if(node.currentSrc) {const key=`r${resources.length}`;resources.push({key,url:node.currentSrc,kind:'image'});record.resource=key;}
        }
        // Pseudo text is literal sanitized text, not a CSS program.
        record.pseudo={};
        for(const pseudo of ['::before','::after']) {
          const content=getComputedStyle(node,pseudo).content;
          if(content && content!=='none' && content!=='normal') {
            if(tainted(content) || !/^"[^"\\]*"$/.test(content)) {record.kind='tile';record.tag='img';record.reason='unsupported_pseudo';return done(record);}
            record.pseudo[pseudo]={text:content.slice(1,-1),style:safeStyle(node,pseudo)};
          }
        }
      }

      for(const child of node.childNodes) {const childId=visit(child,record.id,depth+1);if(childId)record.children.push(childId);}
      if(node.shadowRoot) {const childId=visit(node.shadowRoot,record.id,depth+1);if(childId)record.children.push(childId);}
      return done(record);
    };
    watch(document); const root=visit(document.documentElement);
    if(!variants.length) {
      const usedFamilies=new Set(records.filter(n=>n.kind!=='mask').flatMap(n=>(n.style?.['font-family']??'').split(',').map(s=>s.trim().replaceAll('"','').replaceAll("'",'').toLowerCase())));
      let ruleCount=0;
      const collect = (rules,base) => {
        for(const rule of rules) {
          if(++ruleCount>5000)throw new Error('mirror stylesheet bounds');
          if(rule.type===CSSRule.FONT_FACE_RULE) {
            const source=/url\(["']?([^"')]+)["']?\)/.exec(rule.style.getPropertyValue('src'));
            if(source && fonts.length<32 && usedFamilies.has(rule.style.fontFamily.replaceAll('"','').replaceAll("'",'').trim().toLowerCase())) {
              const url=new URL(source[1],base).href,key=`r${resources.length}`;
              resources.push({key,url,kind:'font'});fonts.push({family:rule.style.fontFamily,weight:rule.style.fontWeight,style:rule.style.fontStyle,resource:key});
            }
          } else if(rule.cssRules) collect(rule.cssRules,base);
        }
      };
      for(const sheet of document.styleSheets) {try{collect(sheet.cssRules,sheet.href??document.baseURI);}catch{}}
    }
    live=nextLive;maskedNodes=new WeakSet(records.filter(n=>n.kind==='mask').map(n=>nextLive.get(n.id)));
    const recordMap=new Map(records.map(n=>[n.id,n])),fontSamples=new Map();
    if(!variants.length)for(const record of records)if(record.kind==='text'&&record.text.trim()||record.reason==='native_control'){
      const parent=record.reason==='native_control'?record:recordMap.get(record.parent);if(parent?.kind!=='element'&&parent?.reason!=='native_control')continue;
      const key=fontKey(parent.style),node=nextLive.get(parent.id),previous=fontSamples.get(key);
      const painted=element=>{for(let e=element;e;e=e.parentElement){const s=getComputedStyle(e);if(s.visibility!=='visible'||Number(s.opacity)===0)return false}return true};
      if(!previous&&fontSamples.size<64||previous&&!painted(previous)&&painted(node))fontSamples.set(key,node);
    }
    fontRepresentatives=[...fontSamples];
    let focused=document.activeElement;for(let i=0;i<128;i++){let child=focused?.shadowRoot?.activeElement;try{child??=focused?.localName==='iframe'?focused.contentDocument?.activeElement:null;}catch{}if(!child||child===focused)break;focused=child;}
    let selection=null;for(const owner of new Set([...nextLive.values()].map(n=>n.ownerDocument??document))){const selected=owner.getSelection();if(!selected||selected.isCollapsed)continue;const anchor=ids.get(selected.anchorNode),focus=ids.get(selected.focusNode);if(records.some(n=>n.id===anchor&&n.kind==='text')&&records.some(n=>n.id===focus&&n.kind==='text'))selection={anchor_id:anchor,anchor_offset:selected.anchorOffset,focus_id:focus,focus_offset:selected.focusOffset};}
    let nodes=records,removed=[],incremental=false;
    if(subscription) {
      const previous=reset?null:snapshots.get(subscription),serialized=serializedRecords;
      if(previous){incremental=true;nodes=records.filter(n=>serialized.get(n.id)!==previous.get(n.id));removed=[...previous.keys()].filter(key=>!serialized.has(key));}
      snapshots.delete(subscription);snapshots.set(subscription,serialized);
      while(snapshots.size>8)snapshots.delete(snapshots.keys().next().value);
    }
    // MP-10: CDP serializes repeated object references by value. Intern the exact
    // sanitized CSS maps privately; public packet/hash shapes remain unchanged.
    const styles=[],styleIds=new Map();
    nodes=nodes.map(node=>{if(!node.style)return node;const key=JSON.stringify(node.style);if(!styleIds.has(key)){styleIds.set(key,styles.length);styles.push(node.style)}const {style,...packed}=node;return {...packed,style_index:styleIds.get(key)};});
    return {animating:[...styleRoots].some(root=>(root.ownerDocument??root).getAnimations().length>0),root,nodes,styles,removed,incremental,resources,fonts,scroll:{x:scrollX,y:scrollY},revision,selection,focused:ids.get(focused)??null};
  };
  // MP-11: an admitted older packet may not retarget an element that moved or
  // changed after the latest sample. Validate again in the real isolated world.
  const validate = (expected,scroll=false) => {
    let geometryChanged=false;
    for(const record of expected??[]) {
      const node=live.get(record.id);
      if(!node?.isConnected||maskedNodes.has(node))throw new Error('mirror changed/protected live target');
      if(record.kind==='text'){if(node.nodeType!==3||node.data!==record.text)throw new Error('mirror changed live text');continue;}
      if(node.nodeType!==1)throw new Error('mirror changed live element');
      const current=box(node);
      if(record.box&&Object.keys(current).some(key=>Math.abs(current[key]-record.box[key])>0.5)){if(!scroll)throw new Error('mirror changed live geometry');geometryChanged=true;}
      for(const [key,value]of Object.entries(record.attributes??{})) {
        const actual=key==='contenteditable'?(['true','false','plaintext-only'].includes(node.contentEditable)?node.contentEditable:(node.isContentEditable?'true':'false')):node.getAttribute(key);
        if(actual!==value)throw new Error('mirror changed live attribute');
      }
    }
    return !geometryChanged;
  };
  const unprotected = node => {
    for(let ancestor=node,depth=0;ancestor&&depth<128;depth++) {
      if(maskedNodes.has(ancestor)||ancestor.matches?.('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')||/password|one-time-code|cc-/i.test(ancestor.autocomplete??'')||protectedVariants.some(v=>[ancestor.value??'',...Array.from(ancestor.attributes??[],a=>a.value)].some(s=>s.includes(v))))throw new Error('mirror protected input ancestor');
      ancestor=ancestor.parentElement??ancestor.getRootNode()?.host??ancestor.ownerDocument?.defaultView?.frameElement;
    }
  };
  const locate = request => {
    const node=live.get(request.node_id);
    if(!node || !node.isConnected || node.nodeType!==1 || node.matches('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]') || /password|one-time-code|cc-/i.test(node.autocomplete??'')) throw new Error('mirror stale/protected node');
    unprotected(node);
    const r=box(node);let x=r.x+r.width/2,y=r.y+r.height/2;
    if(!(r.width>0&&r.height>0&&x>=0&&y>=0&&x<innerWidth&&y<innerHeight)) throw new Error('mirror offscreen node');
    let hit=node.ownerDocument.elementFromPoint(x,y);
    while(hit?.shadowRoot?.elementFromPoint(x,y)) hit=hit.shadowRoot.elementFromPoint(x,y);
    if(hit!==node && !node.contains(hit)) throw new Error('mirror occluded node');
    for(let view=node.ownerDocument.defaultView;view!==window;view=view.parent){const owner=view.frameElement;if(!owner)throw new Error('mirror frame unavailable');if(getComputedStyle(owner).transform!=='none')throw new Error('mirror transformed frame requires coordinates');const b=owner.getBoundingClientRect(),style=getComputedStyle(owner);x+=b.x+owner.clientLeft+(parseFloat(style.paddingLeft)||0);y+=b.y+owner.clientTop+(parseFloat(style.paddingTop)||0);}
    if(x<0||y<0||x>=innerWidth||y>=innerHeight)throw new Error('mirror offscreen frame');
    return {x:Math.floor(x),y:Math.floor(y)};
  };
  const coordinateTarget = (point,opaque=[],expected=[]) => {
    let owner=document,x=point.x,y=point.y,hit;
    for(let depth=0;depth<128;depth++) {
      hit=owner.elementFromPoint(x,y);
      for(let shadow=0;shadow<128&&hit?.shadowRoot;shadow++){const child=hit.shadowRoot.elementFromPoint(x,y);if(!child||child===hit)break;hit=child;}
      if(hit?.localName!=='iframe')break;
      let nested;try{nested=hit.contentDocument;}catch{}if(!nested)break;
      const r=hit.getBoundingClientRect(),style=getComputedStyle(hit);if(style.transform!=='none')throw new Error('mirror transformed coordinate frame');
      x-=r.x+hit.clientLeft+(parseFloat(style.paddingLeft)||0);y-=r.y+hit.clientTop+(parseFloat(style.paddingTop)||0);owner=nested;
    }
    // Native controls/opaque subtrees intentionally omit their descendants.
    let node=hit;const opaqueIds=new Set(opaque);
    for(let ancestor=hit,depth=0;ancestor&&depth<128;depth++){if(opaqueIds.has(ids.get(ancestor))){node=ancestor;break;}ancestor=ancestor.parentElement??ancestor.getRootNode()?.host??ancestor.ownerDocument?.defaultView?.frameElement;}
    while(node&&!ids.has(node)){if(node.parentElement?.matches('button,input,textarea,select')){node=node.parentElement;break;}node=null;}
    const key=ids.get(node);if(!key||!live.has(key)||maskedNodes.has(node)||node.matches('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]')||/password|one-time-code|cc-/i.test(node.autocomplete??''))throw new Error('mirror stale/protected coordinate target');
    // MP-11: a live hit must still be the target observed at this point, and
    // its full element/frame ancestry must retain the sampled geometry.
    unprotected(node);
    if(expected.length) {
      if(key!==expected[0].id){if(point.kind==='scroll')return {scroll_epoch_refused:true};throw new Error('mirror changed live coordinate target');}
      if(!validate(expected,point.kind==='scroll'))return {scroll_epoch_refused:true};
    }
    return key;
  };
  const activeTarget = (expected=[],editable=true) => {
    let node=document.activeElement;
    for(let depth=0;depth<128;depth++) {
      const nested=node?.shadowRoot?.activeElement??(node?.localName==='iframe'?node.contentDocument?.activeElement:null);
      if(!nested)break;node=nested;
    }
    const key=ids.get(node);
    if(!key||!live.has(key)||!node.isConnected||editable&&!node.isContentEditable&&!['input','textarea'].includes(node.localName))throw new Error('mirror unavailable native text focus');
    unprotected(node);locate({node_id:key});
    if(expected.length){if(key!==expected[0].id)throw new Error('mirror changed native text focus');validate(expected);}
    return key;
  };
  const focus = request => {locate(request);const node=live.get(request.node_id);node.focus({preventScroll:true});locate(request);let active=node.ownerDocument.activeElement;while(active?.shadowRoot?.activeElement)active=active.shadowRoot.activeElement;if(active!==node)throw new Error('mirror focus redirected');return true;};
  const select = request => {
    const a=live.get(request.anchor_id),b=live.get(request.focus_id);
    if(!a?.isConnected||!b?.isConnected||a.nodeType!==3||b.nodeType!==3||a.ownerDocument!==b.ownerDocument||!Number.isInteger(request.anchor_offset)||!Number.isInteger(request.focus_offset)||request.anchor_offset<0||request.anchor_offset>a.length||request.focus_offset<0||request.focus_offset>b.length) throw new Error('mirror invalid selection');
    const range=a.ownerDocument.createRange(),pa=a.ownerDocument.createRange(),pb=a.ownerDocument.createRange();pa.setStart(a,request.anchor_offset);pa.collapse(true);pb.setStart(b,request.focus_offset);pb.collapse(true);
    if(pa.compareBoundaryPoints(Range.START_TO_START,pb)>0){range.setStart(b,request.focus_offset);range.setEnd(a,request.anchor_offset);}else{range.setStart(a,request.anchor_offset);range.setEnd(b,request.focus_offset);}
    for(const node of live.values())if(node.ownerDocument===a.ownerDocument && (maskedNodes.has(node) || node.matches?.('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]') || protectedVariants.some(v=>(node.nodeValue??node.value??'').includes(v))) && range.intersectsNode(node))throw new Error('mirror selection intersects protected content');
    const selection=a.ownerDocument.getSelection();selection.setBaseAndExtent(a,request.anchor_offset,b,request.focus_offset);return true;
  };
  // MP-10: private CDP diagnostics expose geometry of already-sanitized text only.
  const textRuns = keys => {
    if(!Array.isArray(keys)||keys.length>12000)throw new Error('mirror geometry bounds');
    return keys.map(key=>{const node=live.get(key);if(!node?.isConnected||node.nodeType!==3||maskedNodes.has(node))throw new Error('mirror unavailable text geometry');const range=node.ownerDocument.createRange();range.selectNodeContents(node);return {id:key,rects:[...range.getClientRects()].map(r=>({x:r.x,y:r.y,width:r.width,height:r.height}))};});
  };
  const customHosts=()=>{const hosts=[],pending=[document.documentElement];let visited=0;while(pending.length){if(++visited>400000)throw Error('mirror custom host memory budget');const node=pending.pop();if(node.nodeType===1&&node.localName.includes('-'))hosts.push(node);pending.push(...node.children??[]);if(node.shadowRoot)pending.push(node.shadowRoot);if(node.localName==='iframe'){let nested;try{nested=node.contentDocument}catch{}if(nested?.documentElement)pending.push(nested.documentElement)}}return hosts.length?hosts:null};
  globalThis.__charioxMirror=Object.freeze({read,customHosts,fontKeys:()=>fontRepresentatives.map(([key])=>key),fontHosts:()=>fontRepresentatives.map(([,node])=>node),admitCustomHost:(node,closed,flow)=>{nativeCustom.set(node,closed?'closed':'light');if(closed&&flow)opaqueFlow.set(node,flow);else opaqueFlow.delete(node)},epoch:()=>{if(observer.takeRecords().length)revision++;if(cssFingerprint!==null){try{if(inspectCss(lastCssRoots,lastCssForms,lastCssCustom,revision,protectedVariants,id)!==cssFingerprint)revision++}catch{revision++}}return revision},locate,focus,select,validate,coordinateTarget,activeTarget,textRuns});return true;
}
