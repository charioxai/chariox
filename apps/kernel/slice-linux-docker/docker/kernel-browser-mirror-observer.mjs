// MP-08/MP-10/MP-11: install only in the controller's #607 isolated world.
// Raw mutation records and page code never cross the mirror boundary.
export const mirrorObserverExpression = initial => `(${installMirrorObserver.toString()})(${JSON.stringify(initial)})`;
function installMirrorObserver(initialStyles = {}) {
  if (globalThis.__charioxMirror) return true;
  const ids = new WeakMap();let observed = new WeakSet();
  let serial = 0, live = new Map(), revision = 0, maskedNodes=new WeakSet(), filledNodes=new WeakSet();
  const observer = new MutationObserver(() => { revision++; });
  const watch = root => {
    if (observed.has(root)) return;
    observed.add(root); observer.observe(root, { subtree:true, childList:true, attributes:true, characterData:true });
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
  const read = (_ignoredValues = [], opaqueRegions = [], subscription = null, reset = false) => {
    observer.disconnect();observed=new WeakSet();
    const records = [], resources = [], fonts = [], nextLive = new Map();
    const serializedRecords=new Map();
    let wireSize=0;const done=record=>{const serialized=JSON.stringify(record);serializedRecords.set(record.id,serialized);wireSize+=serialized.length;if(wireSize>3*1024*1024)throw new Error('mirror snapshot bounds');return record.id;};
    let textSize = 0;
    // MP-08/MP-10/MP-11: exact same-read CSS sharing for plain leaf paragraphs.
    // Same parent, tag, raw attributes, box size and complete selector match set
    // imply the same author cascade + inheritance. Uninspectable/grouped/nested/
    // pseudo CSS, shadows, frames and animations disable it. Nothing persists
    // across reads, so property-only or stylesheet mutations cannot stale it.
    let selectors=null;
    if(!document.getAnimations().length)try {
      selectors=[];
      for(const sheet of [...document.styleSheets,...document.adoptedStyleSheets])for(const rule of sheet.cssRules) {
        if(rule.type===CSSRule.FONT_FACE_RULE)continue;
        if(rule.type!==CSSRule.STYLE_RULE||rule.cssRules?.length||rule.selectorText.includes('::')||selectors.length>=64)throw new Error('mirror CSS sharing unavailable');
        selectors.push(rule.selectorText);
      }
    }catch{selectors=null;}
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
    const visit = (node,parent=null,depth=0) => {
      if(depth>128 || records.length>=12000) throw new Error('mirror node bounds');
      if(![1,3,9,11].includes(node.nodeType)) return null;
      if(node.nodeType===1 && active.has(node.localName)) return null;
      const record={id:id(node),parent,children:[],kind:'element'};
      nextLive.set(record.id,node); records.push(record);
      if(node.nodeType===3) {
        record.kind='text';record.text=node.data;record.box=box(node);
        textSize+=record.text.length; if(textSize>2097152) throw new Error('mirror text bounds');
        return done(record);
      }
      if(node.nodeType===9 || node.nodeType===11) { record.kind=node.nodeType===9?'document':'shadow'; watch(node); }
      else {
        const tag=node.localName;
        record.tag=tags.has(tag)?tag:'div'; record.box=box(node);
        // MP-08/MP-11: CDP binds exact filled nodes across same-renderer frames.
        // Local CSS boxes never overlap root device-pixel rectangles here.
        if(filledNodes.has(node) && (tag==='input'||tag==='textarea'||node.isContentEditable)) {
          record.kind='mask'; record.tag=tags.has(tag)?tag:'div'; record.style={...safeStyle(node,null,false),width:`${record.box.width}px`,height:`${record.box.height}px`,background:'black',color:'transparent','border-color':'black'};
          return done(record);
        }
        record.style=safeStyle(node,null,true,record.box);
        // MP-08/MP-11: show password dots through pixels, never form metadata.
        if(tag==='input'&&node.type==='password'){record.kind='tile';record.reason='native_control';return done(record);}
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
        if(['button','input','textarea','select'].includes(tag) && getComputedStyle(node).appearance!=='none'){record.kind='tile';record.reason='native_control';return done(record);}
        if(media.has(tag) || tag.includes('-') && !node.shadowRoot) {record.kind='tile';record.tag='img';record.reason=tag.includes('-')?'opaque_shadow':'opaque_media';return done(record);}
        if(tag==='iframe') {
          try {
            const nested=node.contentDocument;
            if(!nested?.documentElement) throw new Error('cross origin');
            // Use a separate inert nested document, never its original src/srcdoc.
            record.kind='frame';record.tag='iframe';record.children.push(visit(nested,record.id,depth+1));return done(record);
          } catch {record.kind='tile';record.tag='img';record.reason='cross_origin_frame';return done(record);}
        }
        if(tag==='img') {
          if(node.currentSrc) {const key=`r${resources.length}`;resources.push({key,url:node.currentSrc,kind:'image'});record.resource=key;}
        }
        // Pseudo text is literal sanitized text, not a CSS program.
        record.pseudo={};
        for(const pseudo of ['::before','::after']) {
          const content=getComputedStyle(node,pseudo).content;
          if(content && content!=='none' && content!=='normal') {
            if(!/^"[^"\\]*"$/.test(content)) {record.kind='tile';record.tag='img';record.reason='unsupported_pseudo';return done(record);}
            record.pseudo[pseudo]={text:content.slice(1,-1),style:safeStyle(node,pseudo)};
          }
        }
      }

      for(const child of node.childNodes) {const childId=visit(child,record.id,depth+1);if(childId)record.children.push(childId);}
      if(node.shadowRoot) {const childId=visit(node.shadowRoot,record.id,depth+1);if(childId)record.children.push(childId);}
      return done(record);
    };
    watch(document); const root=visit(document.documentElement);
    {
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
    return {root,nodes,styles,removed,incremental,resources,fonts,scroll:{x:scrollX,y:scrollY},revision,selection,focused:ids.get(focused)??null};
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
