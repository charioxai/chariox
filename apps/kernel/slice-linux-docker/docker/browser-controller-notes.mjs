import { withBrowserFrames } from "./browser-controller-frames.mjs";
// MD-N2 / MP-08 / MP-11: runs ONLY in the #607 controller isolated world.
// No DOM overlay, main-world binding, postMessage, or page-visible note data.
export const NOTE_OBSERVER_EXPRESSION = `(${installNoteObserver.toString()})()`;

function installNoteObserver() {
  if (globalThis.__charioxNotes) return true;
  const maxText = 2 * 1024 * 1024;
  const roots = () => {
    const result=[document.body??document.documentElement];
    for (let i=0;i<result.length;i++) {
      const walker=document.createTreeWalker(result[i],NodeFilter.SHOW_ELEMENT);
      for (let node=walker.nextNode();node;node=walker.nextNode()) if (node.shadowRoot) {
        if (result.length>=64) throw new Error('MD-N2: shadow root limit reached');
        result.push(node.shadowRoot);
      }
    }
    return result;
  };
  const index = (root=document.body??document.documentElement) => {
    const nodes = [];
    let text = "";
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.parentElement?.closest('script,style,noscript,input,textarea,[data-chariox-secret]')) continue;
      const range = document.createRange(); range.selectNodeContents(node);
      if (!range.getClientRects().length) continue;
      if (text.length + node.length > maxText) return null;
      nodes.push({ node, start: text.length }); text += node.data.toWellFormed();
    }
    return { nodes, text };
  };
  const box = range => {
    const r = range.getBoundingClientRect();
    return r.width > 0 && r.height > 0 ? { x:r.x, y:r.y, width:r.width, height:r.height } : null;
  };
  const offset = (nodes, container, value) => {
    let result = 0;
    const point = document.createRange(); point.setStart(container, value); point.collapse(true);
    for (const {node,start} of nodes) {
      if (node === container) return start + value;
      const end = document.createRange(); end.selectNodeContents(node); end.collapse(false);
      if (point.compareBoundaryPoints(Range.START_TO_START, end) >= 0) result = start + node.length;
      else break;
    }
    return result;
  };
  let latest = null;
  const capture = () => {
    latest = null;
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
    let range = selection.getRangeAt(0);
    if (typeof selection.getComposedRanges==='function') {
      const composed=selection.getComposedRanges({shadowRoots:roots().filter(root=>root instanceof ShadowRoot)});
      if (composed.length!==1) return null;
      range=document.createRange();range.setStart(composed[0].startContainer,composed[0].startOffset);range.setEnd(composed[0].endContainer,composed[0].endOffset);
    }
    const root=range.startContainer.getRootNode();
    if (root!==range.endContainer.getRootNode()) return null;
    const indexed = index(root instanceof ShadowRoot?root:document.body??document.documentElement); if (!indexed) return null;
    const start = offset(indexed.nodes, range.startContainer, range.startOffset);
    const end = offset(indexed.nodes, range.endContainer, range.endOffset);
    const exact = indexed.text.slice(start,end);
    if (!exact.isWellFormed() || !exact.trim() || new TextEncoder().encode(exact).length > 16384) return null;
    const rect = box(range); if (!rect) return null;
    latest = { quote: { exact, prefix:Array.from(indexed.text.slice(Math.max(0,start-256),start)).slice(-64).join('').toWellFormed(), suffix:Array.from(indexed.text.slice(end,end+256)).slice(0,64).join('').toWellFormed() }, box_css:rect, hint:JSON.stringify({start,end}) };
    return latest;
  };
  const matchQuote = (quote,root) => {
    const indexed = index(root);
    if (!indexed) throw new Error('MD-N2: page text limit reached');
    const positions=[];
    for (let at=indexed.text.indexOf(quote.exact); at>=0; at=indexed.text.indexOf(quote.exact,at+1)) {
      const end=at+quote.exact.length;
      if ((!quote.prefix || indexed.text.slice(Math.max(0,at-quote.prefix.length),at)===quote.prefix)
          && (!quote.suffix || indexed.text.slice(end,end+quote.suffix.length)===quote.suffix)) positions.push(at);
      if (positions.length>1) break;
    }
    // Changed surroundings may still reattach an unambiguous exact quote.
    if (!positions.length) {
      const at=indexed.text.indexOf(quote.exact);
      if (at>=0 && indexed.text.indexOf(quote.exact,at+1)<0) positions.push(at);
      else if (at>=0) return { anchor_state:'ambiguous', box_css:null };
    }
    if (positions.length!==1) return { anchor_state:positions.length?'ambiguous':'missing', box_css:null };
    const start=positions[0], end=start+quote.exact.length;
    const a=indexed.nodes.find(({node,start:at})=>at+node.length>start);
    const b=indexed.nodes.find(({node,start:at})=>at+node.length>=end);
    if (!a || !b) return { anchor_state:'missing', box_css:null };
    const range=document.createRange(); range.setStart(a.node,start-a.start); range.setEnd(b.node,end-b.start);
    return { anchor_state:'attached', box_css:box(range), hint:JSON.stringify({start,end}) };
  };
  const reanchor = quote => {
    const matches=roots().map(root=>matchQuote(quote,root));
    const attached=matches.filter(result=>result.anchor_state==='attached');
    if (matches.some(result=>result.anchor_state==='ambiguous') || attached.length>1) return {anchor_state:'ambiguous',box_css:null};
    return attached[0]??{anchor_state:'missing',box_css:null};
  };
  document.addEventListener('selectionchange',capture);
  new MutationObserver(()=>{ capture(); }).observe(document.documentElement,{subtree:true,childList:true,characterData:true});
  globalThis.__charioxNotes=Object.freeze({capture,reanchor});
  capture(); return true;
}

export async function observeBrowserNote(browser, request) {
  if (typeof request?.target_id !== 'string' || typeof request?.document_id !== 'string') throw new Error('MD-N2: observed target/document required');
  if (request.quote && (!request.quote.exact || new TextEncoder().encode(request.quote.exact).length>16384 || [request.quote.prefix,request.quote.suffix].some(s=>typeof s!=='string'||new TextEncoder().encode(s).length>512))) throw new Error('MD-N2: invalid quote');
  const connection=await browser.ensureConnection();
  const session=await browser.ensureTargetSession(connection,request.target_id);
  const readFrame=async()=> (await connection.send('Page.getFrameTree',{},session)).frameTree?.frame;
  const frame=await readFrame();
  if (!frame || frame.loaderId!==request.document_id) throw new Error('MD-N2: stale document; refresh tab');
  return withBrowserFrames(connection,session,request.target_id,request.document_id,async entries=>{
    // Reuse the controller's frame ownership channel, including OOPIF renderers.
    const frames=new Map();
    for (const entry of entries) {
      const visit=(tree,parent)=>{
        const item={frame:tree.frame,sessionId:entry.sessionId,parent};
        frames.set(tree.frame.id,item);
        for (const child of tree.childFrames??[]) visit(child,item);
      };
      visit(entry.tree,entry.frame.parentId?frames.get(entry.frame.parentId):null);
    }
    // OOPIF entries replace a parent's placeholder with the owning session.
    for (const item of frames.values()) if (item.frame.parentId) item.parent=frames.get(item.frame.parentId);
    const candidates=[];let ambiguous=false;
    for (const item of frames.values()) {
      if (!item.frame.loaderId) continue;
      const world=item.frame.id===frame.id
        ? await browser.ensureFocusWorld(connection,session,request.target_id,frame)
        : {contextId:(await connection.send('Page.createIsolatedWorld',{frameId:item.frame.id,worldName:'chariox-controller-focus',grantUniveralAccess:false},item.sessionId)).executionContextId};
      if (!Number.isSafeInteger(world.contextId) || world.contextId<=0) throw new Error('MD-N2: isolated world unavailable');
      if (!world.notesInstalled) {
        const installed=await connection.send('Runtime.evaluate',{expression:NOTE_OBSERVER_EXPRESSION,contextId:world.contextId,returnByValue:true},item.sessionId);
        if (installed.exceptionDetails || installed.result?.value!==true) throw new Error('MD-N2: isolated observer unavailable');
        world.notesInstalled=true;
      }
      const expression=request.quote ? `globalThis.__charioxNotes.reanchor(${JSON.stringify(request.quote)})` : 'globalThis.__charioxNotes.capture()';
      const result=await connection.send('Runtime.evaluate',{expression,contextId:world.contextId,returnByValue:true,awaitPromise:false},item.sessionId);
      if (result.exceptionDetails || !Object.hasOwn(result.result??{},'value')) throw new Error('MD-N2: selection observation failed');
      const value=result.result.value;
      if (value?.anchor_state==='ambiguous') ambiguous=true;
      if (!value || (request.quote && value.anchor_state!=='attached')) continue;
      if (value.box_css) value.box_css=await projectFrameBox(connection,item,value.box_css);
      if (value.hint) value.hint=JSON.stringify({frame_id:item.frame.id,document_id:item.frame.loaderId,range:value.hint});
      candidates.push({value,url:item.frame.url});
    }
    // Recheck every owning tree before releasing a selection. A navigation or
    // frame replacement cannot turn a captured box into a new-document anchor.
    for (const entry of entries) {
      const current=await connection.send('Page.getFrameTree',{},entry.sessionId);
      if (JSON.stringify(current.frameTree)!==JSON.stringify(entry.tree)) throw new Error('MD-N2: frame changed during capture');
    }
    const after=await readFrame();
    if (after?.loaderId!==frame.loaderId) throw new Error('MD-N2: document changed during capture');
    const chosen=candidates.length===1 && !ambiguous?candidates[0]:null;
    return { target_id:request.target_id,document_id:frame.loaderId,url:chosen?.url??frame.url,
      selection:request.quote?null:chosen?.value??null,
      anchoring:request.quote?(chosen?.value??{anchor_state:ambiguous||candidates.length>1?'ambiguous':'missing',box_css:null}):null };
  });
}

async function projectFrameBox(connection,item,box) {
  if (!item.parent) return box;
  const owner=await connection.send('DOM.getFrameOwner',{frameId:item.frame.id},item.parent.sessionId);
  const {model}=await connection.send('DOM.getBoxModel',{backendNodeId:owner.backendNodeId},item.parent.sessionId);
  const {executionContextId}=await connection.send('Page.createIsolatedWorld',{frameId:item.frame.id,worldName:'chariox-controller-focus',grantUniveralAccess:false},item.sessionId);
  if (!Number.isSafeInteger(executionContextId)||executionContextId<=0) throw new Error('MD-N2: isolated frame geometry unavailable');
  const viewport=await connection.send('Runtime.evaluate',{expression:'({width:innerWidth,height:innerHeight})',contextId:executionContextId,returnByValue:true},item.sessionId);
  if (viewport.exceptionDetails) throw new Error('MD-N2: isolated frame geometry unavailable');
  const size=viewport.result?.value,quad=model?.content;
  if (!quad || quad.length!==8 || !size || !(size.width>0 && size.height>0)) throw new Error('MD-N2: frame geometry unavailable');
  const points=[];
  for (const [x,y] of [[box.x,box.y],[box.x+box.width,box.y],[box.x+box.width,box.y+box.height],[box.x,box.y+box.height]]) {
    points.push([quad[0]+(quad[2]-quad[0])*x/size.width+(quad[6]-quad[0])*y/size.height,quad[1]+(quad[3]-quad[1])*x/size.width+(quad[7]-quad[1])*y/size.height]);
  }
  const xs=points.map(p=>p[0]),ys=points.map(p=>p[1]);
  return projectFrameBox(connection,item.parent,{x:Math.min(...xs),y:Math.min(...ys),width:Math.max(...xs)-Math.min(...xs),height:Math.max(...ys)-Math.min(...ys)});
}
