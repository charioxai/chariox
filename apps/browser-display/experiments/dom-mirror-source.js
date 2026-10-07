// MD-DISPLAY-02: evaluated only in a CDP isolated world, never the page's world.
(() => {
  const ids = new WeakMap(), nodes = new Map(); let next = 1, dirty = true;
  const idOf = (node) => {
    if (!ids.has(node)) { ids.set(node, next++); nodes.set(ids.get(node), node); }
    return ids.get(node);
  };
  const styles = ['display','position','top','left','right','bottom','z-index','box-sizing','width','height','min-width','min-height','max-width','max-height','margin-top','margin-right','margin-bottom','margin-left','padding-top','padding-right','padding-bottom','padding-left','border-top','border-right','border-bottom','border-left','border-radius','border-collapse','border-spacing','background-color','color','font-family','font-size','font-weight','font-style','line-height','letter-spacing','white-space','text-align','text-decoration','vertical-align','overflow','overflow-x','overflow-y','flex-direction','flex-wrap','flex-grow','flex-shrink','flex-basis','align-items','justify-content','gap','grid-template-columns','grid-template-rows','float','clear','opacity','cursor','list-style-type','object-fit','appearance','visibility'];
  const tags = new Set('html body main header footer nav article section aside div span p h1 h2 h3 h4 h5 h6 pre code a button input textarea select option label form table thead tbody tr td th ul ol li dl dt dd em strong b i small blockquote br hr details summary'.split(' '));
  new MutationObserver(() => { dirty = true; }).observe(document, { subtree: true, childList: true, characterData: true, attributes: true });
  for (const type of ['input','change','scroll']) document.addEventListener(type, () => { dirty = true; }, true);
  const serialize = (node, output, opaque, masked = false) => {
    const id = idOf(node);
    if (node.nodeType === 3) { output.push({ id, text: masked ? '••••' : node.textContent }); return id; }
    if (node.nodeType !== 1) return null;
    const tag = node.localName;
    if (['script','style','link','meta','noscript','template','base','head'].includes(tag)) return null;
    const privateNode = masked || node.hasAttribute('data-md-private') || node.type === 'password' || node.autocomplete?.includes('one-time-code');
    const computed = getComputedStyle(node), style = {};
    for (const prop of styles) {
      const v = computed.getPropertyValue(prop);
      if (v.length<=256 && (/^[a-zA-Z0-9 #.,%\-+'"]*$/.test(v) || /^(rgb|rgba|hsl|hsla)\([0-9., %]+\)$/.test(v))) style[prop] = v;
    }
    const record = { id, tag: tags.has(tag) ? tag : 'div', style, attrs: {}, children: [], sourceId: ['probe','step','add','route','name','submit','foreign','private'].includes(node.id)?node.id:null };
    for (const key of ['type','colspan','rowspan','disabled','dir','lang']) {
      if (node.hasAttribute(key)) record.attrs[key] = node.getAttribute(key);
    }
    if (['input','textarea','select'].includes(tag)) {
      record.value = privateNode ? '' : node.value;
      record.checked = Boolean(node.checked);
    }
    if(node.id==='probe'&&/^\d{1,2}$/.test(node.dataset.seq??''))record.attrs['data-seq']=node.dataset.seq;
    const rect = node.getBoundingClientRect();
    if (['canvas','video','iframe','object','embed','img','svg'].includes(tag)) {
      record.opaque = tag;
      if (rect.width > 0 && rect.height > 0) opaque.push({ id, kind: tag, x: rect.x, y: rect.y, width: rect.width, height: rect.height });
    } else if (!privateNode) {
      record.children = [...node.childNodes].map((child) => serialize(child, output, opaque, false)).filter((x) => x !== null);
    } else { record.children = []; }
    output.push(record); return id;
  };
  globalThis.mdMirror = {
    snapshot(force = false) {
      if (!dirty && !force) return null;
      dirty = false; const records = [], opaque = [];
      const root = serialize(document.body, records, opaque);
      if(records.length>10000 || JSON.stringify(records).length>1024*1024)throw Error('MD-DISPLAY: DOM bound');
      const live = new Set(records.map((r) => r.id));
      for (const id of nodes.keys()) if (!live.has(id)) nodes.delete(id);
      return { root, records, opaque, scrollX, scrollY, dpr: devicePixelRatio, viewport: [innerWidth, innerHeight] };
    },
    target(id) {
      const node = nodes.get(id);
      if (!node || !node.isConnected || node.nodeType !== 1) return null;
      const r = node.getBoundingClientRect();
      return { x: r.x, y: r.y, width: r.width, height: r.height, tag: node.localName, password: node.type === 'password' };
    },
  };
})();
