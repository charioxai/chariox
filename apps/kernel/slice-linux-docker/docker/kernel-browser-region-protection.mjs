// Screenshot-region masks from trusted CDP metadata, never page JavaScript.
async function regions(connection, sessionId, mirrorStructured = false) {
  const { root } = await connection.send("DOM.getDocument", { depth: -1, pierce: true }, sessionId);
  const { nodeIds } = await connection.send("DOM.querySelectorAll", {
    nodeId: root.nodeId,
    selector: 'input[type="password"], [data-chariox-secret], [data-chariox-observation-protected], [data-observation-protected], input[autocomplete*="password" i], input[autocomplete*="one-time-code" i], input[autocomplete*="cc-" i], iframe, frame',
  }, sessionId);
  if (!Array.isArray(nodeIds)) throw new Error("Capture protection unavailable");
  // MP-11: structured mirrors may inspect only same-origin nested documents
  // that CDP actually exposes. Other frames keep opaque protection.
  const admittedFrames=new Set();
  if(mirrorStructured) {
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    const origin=frameTree?.frame?.securityOrigin;
    const admit=tree=>{if(!origin||origin==='://'||tree.frame.securityOrigin!==origin)return;admittedFrames.add(tree.frame.id);for(const child of tree.childFrames??[])admit(child);};
    if(frameTree)admit(frameTree);
  }
  const nodes = [...nodeIds], pending = [root], exposedFrames=new Set(), explicitlyProtected=new Set();
  let visited = 0;
  while (pending.length) {
    if (++visited > 100_000) throw new Error("Capture protection tree limit exceeded");
    const node = pending.pop();
    if (node.shadowRoots?.length) {
      if((!mirrorStructured && node.shadowRoots.some(root=>root.shadowRootType!=='user-agent')) || node.shadowRoots.some(root=>root.shadowRootType==='closed'))nodes.push(node.nodeId);
      if(mirrorStructured)pending.push(...node.shadowRoots.filter(root=>root.shadowRootType==='open'));
    }
    if(mirrorStructured) {
      // Inspect open shadow descendants through trusted CDP metadata, never page
      // scripts. UA shadow roots of ordinary inputs/media are native controls.
      const attrs=new Map();for(let i=0;i<(node.attributes?.length??0);i+=2)attrs.set(node.attributes[i],node.attributes[i+1]);
      if(['data-chariox-secret','data-chariox-observation-protected','data-observation-protected'].some(key=>attrs.has(key)) || node.localName==='input' && (attrs.get('type')?.toLowerCase()==='password'||/password|one-time-code|cc-/i.test(attrs.get('autocomplete')??''))){nodes.push(node.nodeId);explicitlyProtected.add(node.nodeId);}
    }
    if(mirrorStructured&&node.contentDocument&&admittedFrames.has(node.frameId)) {
      exposedFrames.add(node.nodeId);pending.push(node.contentDocument);
    }
    pending.push(...(node.children ?? []));
  }
  if (nodes.length > 1024) throw new Error("Capture protection limit exceeded");
  const result = [];
  for (const nodeId of new Set(nodes.filter(id=>!exposedFrames.has(id)||explicitlyProtected.has(id)))) {
    const { model } = await connection.send("DOM.getBoxModel", { nodeId }, sessionId);
    const quad = model?.border;
    if (!Array.isArray(quad) || quad.length !== 8 || quad.some(n => !Number.isFinite(n))) {
      throw new Error("Capture protection bounds unavailable");
    }
    const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
    result.push({ x: Math.min(...xs), y: Math.min(...ys), width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) });
  }
  return result;
}

export const protectedHostRegions = (connection, sessionId) => regions(connection, sessionId);

export async function captureRegionMasks(connection, sessionId, { mirrorStructured = false } = {}) {
  // Layout changes or failed metadata checks cannot reveal an unmapped field.
  const before = await regions(connection, sessionId, mirrorStructured);
  return { async afterCapture({ width = 1280, height = 800 } = {}) {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error("Capture geometry unavailable");
    const fullFrame = [{ x: 0, y: 0, width, height }];
    try {
      const after = await regions(connection, sessionId, mirrorStructured);
      if (JSON.stringify(before) !== JSON.stringify(after)) return fullFrame;
      // CDP bounds are CSS coordinates; the raster crop masks native PNG pixels.
      return after.map(region => ({ x: region.x * width / 1280, y: region.y * height / 800,
        width: region.width * width / 1280, height: region.height * height / 800 }));
    } catch { return fullFrame; }
  } };
}
