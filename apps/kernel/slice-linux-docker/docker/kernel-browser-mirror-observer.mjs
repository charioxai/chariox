// MP-08/MP-10/MP-11: install only in the controller's #607 isolated world.
// Raw mutation records and page code never cross the mirror boundary.
export const mirrorObserverExpression = initial => `(${installMirrorObserver.toString()})(${JSON.stringify(initial)})`;
function installMirrorObserver(initialStyles = {}) {
  if (globalThis.__charioxMirror) return true;
  const ids = new WeakMap();let observed = new WeakSet();
  let serial = 0, live = new Map(), revision = 0, maskedNodes=new WeakSet(), filledNodes=new WeakSet();
  // MP-10: what changed between two reads. A node whose style inputs did not change keeps its previous
  // style instead of enumerating ~360 computed properties again (that was most of a read).
  const touchedAttributes = new Set(), touchedContent = new Set(), styleCache = new WeakMap(), epochs = new WeakMap();
  let sheetGeneration = 0, touchedAll = false, roots = [document], hot = [new Set(),new Set(),new Set(),new Set()], wasAnimated = new Set(), controls = new Map(), tiles = new Set(), sheetKey = '', sheets = null, fullAt = -Infinity;
  const touch = records => {
    for(const record of records) {
      if(touchedAttributes.size+touchedContent.size>4096){touchedAll=true;return;}
      if(record.type==='attributes')touchedAttributes.add(record.target);
      else {const node=record.target.nodeType===3?record.target.parentNode:record.target;if(node)touchedContent.add(node);}
      // A stylesheet edited, added or removed restyles the whole document.
      if(['style','link'].includes(record.target.localName)||record.target.parentNode?.localName==='style'||[...record.addedNodes,...record.removedNodes].some(node=>['style','link'].includes(node.localName)))touchedAll=true;
    }
  };
  // Stylesheets are read once per shape: a sheet added, removed, disabled or given a rule, or a new viewport.
  // `plain` = cached styles can be trusted (every sheet readable, no @scope/@starting-style); `adjacent` = some selector reads an earlier sibling's attributes or state (`+`, `~`), so such a change also restyles the siblings after it; `structural` = some selector reads positions or emptiness too (:first-child, :empty ...), so added/removed children do as well; before:
  // it; `has` = compounds that host a :has() (a change below them restyles them; `hasSibling`: or beside them); `subject[hover, active, focus]` =
  // compounds that carry that state in some author selector: when such an element changes state it is restyled with
  // its subtree (descendants inherit from it). Other elements are restyled for the user-agent's own state rules only.
  const sheetShape = () => {
    let key=`${innerWidth}x${innerHeight}@${devicePixelRatio}`;
    for(const root of roots){key+='|';try{for(const sheet of [...root.styleSheets,...root.adoptedStyleSheets])key+=`${sheet.disabled?'-':''}${sheet.cssRules.length},`;}catch{key+='?';}}
    return key;
  };
  const states=[/:hover/g,/:active/g,/:(?:focus-visible|focus(?!-within)|target)\b/g,/:focus-within/g], hasGroup=/:has\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)/g;
  // The compounds of every selector in a list (top-level combinators only), each with the combinator before it.
  const compoundsOf = text => {
    const selectors=[[]];let current='',separator='',depth=0,bracket=0,quote='';
    const flush=()=>{if(current)selectors.at(-1).push({text:current,separator});if(current)separator='';current='';};
    for(const c of text) {
      if(quote){current+=c;if(c===quote)quote='';continue;}
      if(c==='"'||c==="'"){quote=c;current+=c;continue;}
      if(c==='(')depth++;else if(c===')')depth--;else if(c==='[')bracket++;else if(c===']')bracket--;
      if(!depth&&!bracket&&c===','){flush();selectors.push([]);separator='';}
      else if(!depth&&!bracket&&/[\s>+~]/.test(c)){flush();separator+=c;}
      else current+=c;
    }
    flush();
    return selectors;
  };
  const scanSheets = () => {
    const found={plain:true,adjacent:false,structural:false,hasSibling:false,has:[],subject:[[],[],[],[]],selectors:null,fonts:[]};
    let count=0;
    // A state inside :not() is dropped with the whole :not(): removing a constraint there would narrow the match.
    const withoutNegations=text=>{
      let out='',at=0;
      for(let open=text.indexOf(':not(');open>=0;open=text.indexOf(':not(',at)) {
        let depth=0,end=open+4;
        do{depth+=text[end]==='('?1:text[end]===')'?-1:0;end++;}while(depth>0&&end<text.length);
        out+=text.slice(at,open)+(/:(?:hover|active|focus|target)/.test(text.slice(open,end))?'':text.slice(open,end));
        at=end;
      }
      return out+text.slice(at);
    };
    const tidy=text=>{for(let pass=0;pass<4;pass++)text=text.replaceAll(/\(\s*,\s*/g,'(').replaceAll(/\s*,\s*(?=\))/g,'').replaceAll(/,\s*,/g,',').replaceAll(/:[\w-]+\(\s*\)/g,'');return text;};
    const analyze=text=>{
      if(/:has\(\s*[+~]/.test(text))found.hasSibling=true;
      const bare=/:(?:hover|active|focus(?:-within|-visible)?|target)\b/g;
      for(const compounds of compoundsOf(text))compounds.forEach(({text:raw},index)=>{
        // States inside a :has() argument are covered by restyling the :has() host. Whatever is inside other parentheses
        // (:is(a:hover), :not(:hover)) cannot be told apart from the compound: all of it.
        const compound=raw.replaceAll(hasGroup,'');
        const top=compound.replaceAll(/\([^()]*\)/g,'').replaceAll(/\([^()]*\)/g,'');
        // The elements that can carry the state or host the :has(): the selector up to this compound, state removed.
        const clean=part=>tidy(withoutNegations(part.replaceAll(hasGroup,'')).replaceAll(bare,''))||'*';
        const base=compounds.slice(0,index+1).map(({text:part,separator})=>separator+clean(part)).join('').trim();
        if(/:has\(/.test(raw))found.has.push(/:has\b/.test(raw.replaceAll(/\([^()]*\)/g,'').replaceAll(/\([^()]*\)/g,''))?base:'*');
        states.forEach((state,kind)=>{
          if(!compound.match(state))return;
          // The state sits in a functional pseudo-class: the element is still this compound's, unless the group reaches
          // another element through a combinator.
          if(compound.match(state).length===(top.match(state)?.length??0))return found.subject[kind].push(base);
          const reaches=(compound.match(/\([^()]*\)/g)??[]).some(group=>group.match(state)&&group.slice(1,-1).split(',').some(alternative=>/[>+~]|\S\s+\S/.test(alternative.trim())));
          found.subject[kind].push(reaches?'*':base);
        });
      });
    };
    const walk=rules=>{
      for(const rule of rules){
        if(++count>100000){found.plain=false;return;}
        if(rule.selectorText!==undefined){
          const plain=rule.selectorText.replaceAll(hasGroup,'');
          if(/[+~]|:nth-[\w-]+\([^)]*\bof\b/.test(plain))found.adjacent=found.structural=true;
          else if(/:(?:first|last|only|nth)-|:empty/.test(plain))found.structural=true;
          if(/:(?:has|hover|active|focus|target)/.test(rule.selectorText))analyze(rule.selectorText);
        } else if(['CSSScopeRule','CSSStartingStyleRule'].includes(rule.constructor.name))found.plain=false;
        if(rule.cssRules)walk(rule.cssRules);
        if(rule.styleSheet)walk(rule.styleSheet.cssRules);
      }
    };
    for(const root of roots)try{for(const sheet of [...root.styleSheets,...root.adoptedStyleSheets])walk(sheet.cssRules);}catch{found.plain=false;}
    // A base the engine cannot parse is widened by dropping its functional pseudo-classes, else it stands for everything.
    const parses=base=>{try{document.createDocumentFragment().querySelector(base);return true;}catch{return false;}};
    const valid=base=>{if(parses(base))return base;const wide=base.replaceAll(/:[\w-]+\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)/g,'').replaceAll(/:[\w-]+\(\)/g,'')||'*';return parses(wide)?wide:'*';};
    const list=bases=>bases.length?[...new Set(bases.map(valid))].join(','):null;
    found.has=list(found.has);found.subject=found.subject.map(list);
    // MP-08/MP-10/MP-11: exact same-read CSS sharing for plain leaf paragraphs.
    // Same parent, tag, raw attributes, box size and complete selector match set
    // imply the same author cascade + inheritance. Uninspectable/grouped/nested/
    // pseudo CSS, shadows, frames and animations disable it.
    try {
      found.selectors=[];
      for(const sheet of [...document.styleSheets,...document.adoptedStyleSheets])for(const rule of sheet.cssRules) {
        if(rule.type===CSSRule.FONT_FACE_RULE)continue;
        if(rule.type!==CSSRule.STYLE_RULE||rule.cssRules?.length||rule.selectorText.includes('::')||found.selectors.length>=64)throw new Error('mirror CSS sharing unavailable');
        found.selectors.push(rule.selectorText);
      }
    }catch{found.selectors=null;}
    let ruleCount=0;
    const collect = (rules,base) => {
      for(const rule of rules) {
        if(++ruleCount>5000)throw new Error('mirror stylesheet bounds');
        if(rule.type===CSSRule.FONT_FACE_RULE) {
          const source=/url\(["']?([^"')]+)["']?\)/.exec(rule.style.getPropertyValue('src'));
          if(source)found.fonts.push({family:rule.style.fontFamily,weight:rule.style.fontWeight,style:rule.style.fontStyle,source:source[1],base});
        } else if(rule.cssRules) collect(rule.cssRules,base);
      }
    };
    for(const sheet of document.styleSheets) {try{collect(sheet.cssRules,sheet.href??document.baseURI);}catch{}}
    return found;
  };
  const observer = new MutationObserver(records => { revision++; touch(records); });
  // Chromium answers :hover and :active as never matching in querySelectorAll()/matches(), so the pointer
  // chains are followed through the events that change them (the listeners are private to this world).
  const pointer = new WeakMap();
  const track = event => {
    const state=pointer.get(event.currentTarget)??{hover:[],active:[]},chain=event.composedPath().filter(node=>node.nodeType===1);
    if(event.type==='mouseover')state.hover=chain;
    else if(event.type==='mouseout'&&!event.relatedTarget)state.hover=[];
    else if(event.type==='mousedown')state.active=chain;
    else if(event.type==='mouseup')state.active=[];
    pointer.set(event.currentTarget,state);
  };
  const watch = root => {
    if (observed.has(root)) return;
    observed.add(root); observer.observe(root, { subtree:true, childList:true, attributes:true, characterData:true });
    if(root.nodeType===9)for(const type of ['mouseover','mouseout','mousedown','mouseup'])root.addEventListener(type,track,{capture:true,passive:true});
  };
  const id = node => { if (!ids.has(node)) ids.set(node, `n${++serial}`); return ids.get(node); };
  const tags = new Set('html head body div span p a article section main header footer nav aside h1 h2 h3 h4 h5 h6 ul ol li dl dt dd pre code blockquote b strong i em u s small sub sup br hr table thead tbody tfoot tr th td caption colgroup col input textarea button select option optgroup label fieldset legend form details summary dialog img figure figcaption picture source slot'.split(' '));
  const attributes = new Set('title alt role aria-label aria-hidden aria-expanded aria-checked aria-selected aria-disabled slot dir lang colspan rowspan span type placeholder disabled readonly multiple size rows cols wrap open start reversed value checked selected'.split(' '));
  const active = new Set('script style link meta base noscript template'.split(' '));
  const media = new Set('canvas video audio svg object embed applet'.split(' '));
  const forbiddenCss = /url\s*\(|image-set\s*\(|(?:-webkit-)?image\s*\(|expression\s*\(|@|\\|[<>]|[\u0000-\u0008]/i;
  const box = node => {
    const r = node.nodeType === 1 ? node.getBoundingClientRect() : (() => { const range=document.createRange(); range.selectNodeContents(node); return range.getBoundingClientRect(); })();
    return {x:r.x,y:r.y,width:r.width,height:r.height};
  };
  const snapshots = new Map();
  // Form state the DOM does not reflect in attributes but selectors can match (:checked, :placeholder-shown, :valid ...).
  const controlState = node => [node.checked,node.indeterminate,node.disabled,node.selected,node.selectedIndex,node.value?.length>0,node.validity?.valid,node.hasAttribute('popover')&&node.matches(':popover-open')].join();
  const read = (_ignoredValues = [], opaqueRegions = [], subscription = null, reset = false) => {
    touch(observer.takeRecords());observer.disconnect();observed=new WeakSet();
    const records = [], resources = [], fonts = [], nextLive = new Map(), tileStates = {}, nextRoots = [], nextControls = new Map(), nextTiles = new Set();
    // MP-10: decide which nodes need their style evaluated again. Anything not listed keeps the style of the
    // previous read, provided the sheets, the viewport and the node's box are unchanged and nothing animates.
    const now=performance.now(),lastFull=fullAt,key=sheetShape(),shaped=key===sheetKey;fullAt=-Infinity;
    if(!shaped){sheets=scanSheets();sheetKey=key;}
    const hovered=new Set(),pressed=new Set(),focusing=new Set(),within=new Set(),dirtyDeep=new Set(),dirtySelf=new Set();
    for(const root of roots) {
      for(const node of pointer.get(root)?.hover??[])if(node.isConnected)hovered.add(node);
      for(const node of pointer.get(root)?.active??[])if(node.isConnected)pressed.add(node);
      try{
        for(const node of root.querySelectorAll(':focus,:focus-visible,:target'))focusing.add(node);
        if(sheets.subject[3])for(const node of root.querySelectorAll(':focus-within'))within.add(node);
      }catch{}
    }
    const hotNow=[hovered,pressed,focusing,within];
    for(const tile of tiles)if([...touchedAttributes,...touchedContent].some(node=>tile.contains(node)))epochs.set(tile,(epochs.get(tile)??0)+1);
    // Whatever animates is restyled every read (and once more when it stops); an animation without an element target is unknowable.
    const animations=roots.flatMap(root=>root.getAnimations()).filter(animation=>!animation.effect||animation.effect.getKeyframes().some(frame=>Object.keys(frame).some(key=>!['offset','easing','composite','computedOffset'].includes(key)))),animated=new Set(animations.map(animation=>animation.effect?.target));
    const lastAnimated=wasAnimated;wasAnimated=animated;
    let quick=!reset&&!touchedAll&&shaped&&sheets.plain&&now-lastFull<=1500&&!animated.has(undefined)&&!animated.has(null);
    if(quick) {
      // A change restyles its node, the subtree below it and (when selectors reach siblings) the siblings after it,
      // up through the hosts of any shadow trees it sits in.
      const restyle=(node,deep,siblings)=>{
        while(node) {
          const host=node.getRootNode().host;
          if(deep||host){for(let sibling=node;sibling;sibling=siblings?sibling.nextElementSibling:null)dirtyDeep.add(sibling);}
          else dirtySelf.add(node);
          node=host;deep=true;
        }
      };
      // Whatever hosts a :has() above the change is restyled with its subtree.
      let scanned=0;
      const mark=(node,deep,siblings)=>{
        restyle(node,deep,siblings);
        markHosts(node);
      };
      const markHosts=node=>{
        if(sheets.has)for(let host=node.parentElement?.closest(sheets.has);host;host=host.parentElement?.closest(sheets.has))restyle(host,true,sheets.adjacent);
        // :has(+ x) / :has(~ x): the hosts are preceding siblings of the changed node or of one of its ancestors.
        if(sheets.hasSibling)for(let ancestor=node;ancestor;ancestor=ancestor.parentElement)for(let before=ancestor.previousElementSibling;before;before=before.previousElementSibling)if(++scanned<=2000&&before.matches(sheets.has))restyle(before,true,sheets.adjacent);
      };
      // What a node changing state restyles: 2 = itself and its subtree (an author rule uses the state on it, or the
      // user-agent colours a pressed link), 1 = itself (the user-agent's rules for focus, and for form controls),
      // 0 = nothing (ancestors of the pointer that no rule asks about).
      const uaControls = 'input,select,textarea,button,summary,option,optgroup,details,video,audio,label,fieldset';
      const effect=(kind,node)=>{
        try{
          if(sheets.subject[kind]&&node.matches(sheets.subject[kind]))return 2;
          if(kind===2)return 1;
          if(kind===3)return 0;
          return kind===1&&node.matches('a,area')?2:node.matches(uaControls)?1:0;
        }catch{return 2;}
      };
      for(const node of touchedAttributes)if(node.isConnected)mark(node,true,sheets.adjacent);
      for(const node of touchedContent)if(node.isConnected)mark(node,sheets.structural,sheets.adjacent);
      hotNow.forEach((now,kind)=>{
        for(const node of [...now].filter(node=>!hot[kind].has(node)).concat([...hot[kind]].filter(node=>!now.has(node)&&node.isConnected))) {
          const reach=effect(kind,node);
          if(reach)mark(node,reach===2,sheets.adjacent);
          else markHosts(node);
        }
      });
      for(const node of [...animated,...lastAnimated])if(node?.isConnected)restyle(node,true,false);
      for(const [node,was] of controls)if(node.isConnected&&controlState(node)!==was)mark(node,true,sheets.adjacent);
      quick=dirtyDeep.size+dirtySelf.size<=4096&&scanned<=2000;
    }
    if(!shaped||touchedAll)sheetGeneration++;
    touchedAttributes.clear();touchedContent.clear();touchedAll=false;hot=hotNow;
    // Private to the kernel (never in a packet): what a native control's pixels depend on beyond its record.
    // Its subtree is not visited: a mutation below it (counted per tile), a stylesheet change, an animation inside it
    // and its pseudo-element content all repaint it without changing its record.
    const tileDamage = node => `${epochs.get(node)??0}.${sheetGeneration}`, animatedInside = node => animations.some(animation=>animation.effect?.target&&node.contains(animation.effect.target));
    const nativeTile = (record,node) => {
      const secret=node.localName==='input'&&node.type==='password',text=node.textContent;
      nextTiles.add(node);
      tileStates[record.id]=(animatedInside(node)?'~':'')+`${+hovered.has(node)}${+pressed.has(node)}`+[':focus',':focus-visible',':disabled',':checked',':indeterminate'].map(s=>node.matches(s)?1:0).join('')+`|${secret?`${node.value.length},${node.selectionStart},${node.selectionEnd}`:''}|${node.childElementCount},${text.length},${text.slice(0,256)}|${tileDamage(node)}|${[...node.querySelectorAll('*')].slice(0,32).map(child=>{const c=getComputedStyle(child);return [c.color,c.backgroundColor,c.opacity,c.visibility,c.display,c.borderTopColor,c.textDecorationColor].join()}).join(';')}|${['::before','::after'].map(pseudo=>getComputedStyle(node,pseudo).content).join('|')}`;
      record.kind='tile';record.reason='native_control';return done(record);
    };
    // Private to the kernel: what an opaque tile's pixels depend on that the isolated world can observe.
    // '=' states are exact (a video's playback state, a subtree's mutation epoch, hover/focus); '~' states
    // cover pixels nothing here can see change (canvas drawing, plugin and cross-origin frame content).
    const opaqueTile = (record,node,tag) => {
      const exact=['video','audio','svg'].includes(tag)||record.reason==='unsupported_pseudo';
      nextTiles.add(node);
      tileStates[record.id]=`${exact&&!animatedInside(node)?'=':'~'}${+hovered.has(node)}${+node.matches(':focus')}|${tileDamage(node)}|${tag==='video'||tag==='audio'?`${node.currentTime}|${+node.paused}${+node.ended}${+node.seeking}|${node.readyState}|${node.videoWidth}x${node.videoHeight}|${node.currentSrc}`:tag==='canvas'?`${node.width}x${node.height}`:''}`;
    };
    const serializedRecords=new Map();
    let wireSize=0;const done=record=>{const serialized=JSON.stringify(record);serializedRecords.set(record.id,serialized);wireSize+=serialized.length;if(wireSize>3*1024*1024)throw new Error('mirror snapshot bounds');return record.id;};
    let textSize = 0;
    const selectors=animations.length?null:sheets.selectors;
    const sharedStyles=new Map();
    const safeStyle = (element,pseudo=null,resourcesAllowed=true,bounds=null) => {
      const computed=getComputedStyle(element,pseudo), out={all:'initial'};
      let sharedKey=null;
      if(selectors&&!pseudo&&bounds&&element.ownerDocument===document&&element.getRootNode()===document&&element.localName==='p'&&element.attributes.length<=16&&[...element.attributes].every(a=>a.value.length<=2048)&&element.childNodes.length===1&&element.firstChild.nodeType===3&&computed.backgroundImage==='none') {
        sharedKey=JSON.stringify([id(element.parentNode),element.localName,[...element.attributes].map(a=>[a.name,a.value]),bounds.width,bounds.height,selectors.map(selector=>element.matches(selector))]);
        if(sharedStyles.has(sharedKey))return {...sharedStyles.get(sharedKey)};
      }
      for (const property of computed) {
        if(property.startsWith('--') || property.startsWith('animation') || property.startsWith('transition') || ['content','cursor'].includes(property)) continue;
        const value=computed.getPropertyValue(property);
        if(value===initialStyles[property] && !['direction','unicode-bidi','color','background-color','border-top-color','border-right-color','border-bottom-color','border-left-color'].includes(property))continue;
        if(value.length<=2048 && !forbiddenCss.test(value)) out[property]=value;
      }
      // Inline URLs are never shipped. Computed image URLs become kernel resource refs.
      if (!pseudo && resourcesAllowed) {
        const value=computed.backgroundImage, match=/^url\("([^"\n]+)"\)$/.exec(value);
        if(match) { const key=`r${resources.length}`; resources.push({key,url:match[1],kind:'image'}); out['background-image']=`resource:${key}`; }
      }
      if(sharedKey)sharedStyles.set(sharedKey,out);
      return out;
    };
    // The style of an element is reused while nothing that feeds it changed (see the plan above) and its box,
    // which carries the layout-dependent used values, is the same. Styles holding resource references are
    // numbered per read and never reused.
    let evaluated=false;
    const elementStyle = (node,box,forced) => {
      const hit=forced?null:styleCache.get(node);
      evaluated=false;
      // Only a positioned element's used offsets follow where it is; scrolling moves everything else without restyling it.
      if(hit&&hit.width===box.width&&hit.height===box.height&&(!hit.style.position||hit.x===box.x&&hit.y===box.y))return hit.style;
      evaluated=true;
      const before=resources.length,style=safeStyle(node,null,true,box);
      if(resources.length===before)styleCache.set(node,{x:box.x,y:box.y,width:box.width,height:box.height,style});else styleCache.delete(node);
      return style;
    };
    const visit = (node,parent=null,depth=0,inherited=false) => {
      if(depth>128 || records.length>=12000) throw new Error('mirror node bounds');
      if(![1,3,9,11].includes(node.nodeType)) return null;
      if(node.nodeType===1 && active.has(node.localName)) return null;
      let forced=inherited||!quick||dirtyDeep.has(node);const record={id:id(node),parent,children:[],kind:'element'};
      nextLive.set(record.id,node); records.push(record);
      if(node.nodeType===3) {
        record.kind='text';record.text=node.data;record.box=box(node);
        textSize+=record.text.length; if(textSize>2097152) throw new Error('mirror text bounds');
        return done(record);
      }
      if(node.nodeType===9 || node.nodeType===11) { record.kind=node.nodeType===9?'document':'shadow'; watch(node); nextRoots.push(node); }
      else {
        const tag=node.localName;
        record.tag=tags.has(tag)?tag:'div'; record.box=box(node);
        // MP-08/MP-11: CDP binds exact filled nodes across same-renderer frames.
        // Local CSS boxes never overlap root device-pixel rectangles here.
        if(filledNodes.has(node) && (tag==='input'||tag==='textarea'||node.isContentEditable)) {
          record.kind='mask'; record.tag=tags.has(tag)?tag:'div'; record.style={...safeStyle(node,null,false),width:`${record.box.width}px`,height:`${record.box.height}px`,background:'black',color:'transparent','border-color':'black'};
          return done(record);
        }
        record.style=elementStyle(node,record.box,forced||dirtySelf.has(node));
        // A size container whose box changed restyles what it contains (@container); `forced` flows to the children below.
        if(evaluated&&record.style['container-type'])forced=true;
        // MP-08/MP-11: show password dots through pixels, never form metadata.
        if(tag==='input'&&node.type==='password')return nativeTile(record,node);
        // MP-08/MP-10: viewport first. Offscreen simple flow blocks retain their
        // layout but hydrate descendants on the following bounded credit.
        // Exact filled-field protection precedes deferral; user scrolling affects
        // only the source until the next packet restores the real subtree.
        if(reset&&record.box.y>=innerHeight&&['p','pre','li'].includes(tag)&&record.style.display==='block'&&(record.style.position??'static')==='static'&&!node.shadowRoot) {
          record.kind='tile';record.reason='viewport_deferred';return done(record);
        }
        record.attributes={};
        for(const attr of node.attributes) if((attributes.has(attr.name)||tag==='slot'&&attr.name==='name') && attr.value.length<=2048) record.attributes[attr.name]=attr.value;
        // MP-08/MP-11: preserve inert editing semantics, never arbitrary values.
        if(node.hasAttribute('contenteditable'))record.attributes.contenteditable=['true','false','plaintext-only'].includes(node.contentEditable)?node.contentEditable:(node.isContentEditable?'true':'false');
        // No name/id/data-* attributes, URLs, event handlers, provider/page secrets.
        if(tag==='input' && !['password','text','search','email','url','number','tel','checkbox','radio','range','button','submit','reset','date','time','color','hidden'].includes(record.attributes.type??'text')) record.attributes.type='text';
        if(['input','textarea','select'].includes(tag)&&typeof node.value==='string'&&node.value.length>16384)throw new Error('mirror form bounds');
        if(tag==='input' || tag==='textarea' || tag==='select') record.form={value:tag==='input'&&node.type==='password'?'':(node.value??'').slice(0,16384),checked:!!node.checked,selected_index:node.selectedIndex??-1,selection_start:node.selectionStart??null,selection_end:node.selectionEnd??null};
        record.scroll={x:node.scrollLeft,y:node.scrollTop};
        if(['input','textarea','select','option'].includes(tag)||node.hasAttribute('popover'))nextControls.set(node,controlState(node));
        if(['button','input','textarea','select'].includes(tag) && getComputedStyle(node).appearance!=='none')return nativeTile(record,node);
        if(media.has(tag) || tag.includes('-') && !node.shadowRoot) {record.kind='tile';record.tag='img';record.reason=tag.includes('-')?'opaque_shadow':'opaque_media';opaqueTile(record,node,tag);return done(record);}
        if(tag==='iframe') {
          try {
            const nested=node.contentDocument;
            if(!nested?.documentElement) throw new Error('cross origin');
            // Use a separate inert nested document, never its original src/srcdoc.
            record.kind='frame';record.tag='iframe';record.children.push(visit(nested,record.id,depth+1,forced));return done(record);
          } catch {record.kind='tile';record.tag='img';record.reason='cross_origin_frame';opaqueTile(record,node,tag);return done(record);}
        }
        if(tag==='img') {
          if(node.currentSrc) {const key=`r${resources.length}`;resources.push({key,url:node.currentSrc,kind:'image'});record.resource=key;}
        }
        // Pseudo text is literal sanitized text, not a CSS program.
        record.pseudo={};
        for(const pseudo of ['::before','::after']) {
          const content=getComputedStyle(node,pseudo).content;
          if(content && content!=='none' && content!=='normal') {
            if(!/^"[^"\\]*"$/.test(content)) {record.kind='tile';record.tag='img';record.reason='unsupported_pseudo';opaqueTile(record,node,tag);return done(record);}
            record.pseudo[pseudo]={text:content.slice(1,-1),style:safeStyle(node,pseudo)};
          }
        }
      }

      for(const child of node.childNodes) {const childId=visit(child,record.id,depth+1,forced);if(childId)record.children.push(childId);}
      if(node.shadowRoot) {const childId=visit(node.shadowRoot,record.id,depth+1,forced);if(childId)record.children.push(childId);}
      return done(record);
    };
    watch(document); nextRoots.push(document); const root=visit(document.documentElement);
    {
      const usedFamilies=new Set(records.filter(n=>n.kind!=='mask').flatMap(n=>(n.style?.['font-family']??'').split(',').map(s=>s.trim().replaceAll('"','').replaceAll("'",'').toLowerCase())));
      for(const font of sheets.fonts)if(fonts.length<32&&usedFamilies.has(font.family.replaceAll('"','').replaceAll("'",'').trim().toLowerCase())) {
        const url=new URL(font.source,font.base).href,key=`r${resources.length}`;
        resources.push({key,url,kind:'font'});fonts.push({family:font.family,weight:font.weight,style:font.style,resource:key});
      }
    }
    roots=nextRoots;controls=nextControls;tiles=nextTiles;fullAt=quick?lastFull:now;
    {const after=sheetShape();if(after!==key)sheets=scanSheets();sheetKey=after;}
    live=nextLive;maskedNodes=new WeakSet(records.filter(n=>n.kind==='mask').map(n=>nextLive.get(n.id)));
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
    return {root,nodes,styles,removed,incremental,resources,fonts,scroll:{x:scrollX,y:scrollY},revision,selection,focused:ids.get(focused)??null,tile_states:tileStates};
  };
  // MP-11: an admitted older packet may not retarget an element that moved or
  // changed after the latest sample. Validate again in the real isolated world.
  const validate = expected => {
    for(const record of expected??[]) {
      const node=live.get(record.id);
      if(!node?.isConnected||maskedNodes.has(node))throw new Error('mirror changed/protected live target');
      if(record.kind==='text'){if(node.nodeType!==3||node.data!==record.text)throw new Error('mirror changed live text');continue;}
      if(node.nodeType!==1)throw new Error('mirror changed live element');
      const current=box(node);
      if(record.box&&Object.keys(current).some(key=>Math.abs(current[key]-record.box[key])>0.5))throw new Error('mirror changed live geometry');
      for(const [key,value]of Object.entries(record.attributes??{})) {
        const actual=key==='contenteditable'?(['true','false','plaintext-only'].includes(node.contentEditable)?node.contentEditable:(node.isContentEditable?'true':'false')):node.getAttribute(key);
        if(actual!==value)throw new Error('mirror changed live attribute');
      }
    }
    return true;
  };
  const unprotected = node => {
    for(let ancestor=node,depth=0;ancestor&&depth<128;depth++) {
      if(maskedNodes.has(ancestor)||ancestor.matches?.('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')||/password|one-time-code|cc-/i.test(ancestor.autocomplete??''))throw new Error('mirror protected input ancestor');
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
  const coordinateTarget = (point,opaque=[],expected=[],addressed=false) => {
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
    // MP-11: element clicks may hit a styled child of the addressed control.
    // Check the actual dispatch point through frames/shadows, including the
    // hit child's live protection, then bind it to the observed control.
    if(addressed) {
      const target=live.get(expected[0]?.id);
      if(!target?.isConnected||hit!==target&&!target.contains(hit))throw new Error('mirror changed addressed click hit');
      unprotected(hit);node=target;
    }
    const key=ids.get(node);if(!key||!live.has(key)||maskedNodes.has(node)||node.matches('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]')||/password|one-time-code|cc-/i.test(node.autocomplete??''))throw new Error('mirror stale/protected coordinate target');
    // MP-11: a live hit must still be the target observed at this point, and
    // its full element/frame ancestry must retain the sampled geometry.
    if(expected.length) {
      if(key!==expected[0].id)throw new Error('mirror changed live coordinate target');
      validate(expected);
    }
    unprotected(node);
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
    for(const node of live.values())if(node.ownerDocument===a.ownerDocument && (maskedNodes.has(node) || node.matches?.('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected],input[type=password]')) && range.intersectsNode(node))throw new Error('mirror selection intersects protected content');
    const selection=a.ownerDocument.getSelection();selection.setBaseAndExtent(a,request.anchor_offset,b,request.focus_offset);return true;
  };
  // MP-10: private CDP diagnostics expose geometry of already-sanitized text only.
  const textRuns = keys => {
    if(!Array.isArray(keys)||keys.length>12000)throw new Error('mirror geometry bounds');
    return keys.map(key=>{const node=live.get(key);if(!node?.isConnected||node.nodeType!==3||maskedNodes.has(node))throw new Error('mirror unavailable text geometry');const range=node.ownerDocument.createRange();range.selectNodeContents(node);return {id:key,rects:[...range.getClientRects()].map(r=>({x:r.x,y:r.y,width:r.width,height:r.height}))};});
  };
  const resetFillTargets=()=>{filledNodes=new WeakSet();return true;};
  const addFillTarget=node=>{filledNodes.add(node);return true;};
  globalThis.__charioxMirror=Object.freeze({read,locate,focus,select,validate,coordinateTarget,activeTarget,textRuns,resetFillTargets,addFillTarget});return true;
}
