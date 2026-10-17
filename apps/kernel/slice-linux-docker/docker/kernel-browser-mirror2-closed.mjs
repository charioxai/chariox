// MP-08/MP-10/MP-11 M2: bounded trusted closed-root discovery. Never serialize
// the entire document into a CDP reply just to inspect a few mirrored hosts.
import { randomUUID } from 'node:crypto';

export async function discoverClosedHosts(mirror, world, source) {
  if (!await mirror.evaluate(world, `${world.ref}.customHosts()`)) return 0;
  const fresh = new Set([...(source?.nodes ?? []), ...(source?.ops ?? []).flatMap(op => op.op === 'children' ? op.nodes : [])].map(r => r.id));
  const replacements = new Map(), removed = new Set(); let published = 0;
  while (await mirror.evaluate(world, `${world.ref}.customHosts()`)) {
    const objectGroup = `chariox-mirror2-closed:${randomUUID()}`;
    const array = await world.connection.send('Runtime.evaluate', { expression: `${world.ref}.customCandidates()`, contextId: world.contextId, objectGroup, returnByValue: false }, world.sessionId);
    const objectId = array.result?.objectId;
    if (!objectId || array.exceptionDetails) throw new Error('MP-11: closed-root candidates unavailable');
    try {
      const { result } = await world.connection.send('Runtime.getProperties', { objectId, ownProperties: true }, world.sessionId);
      const hosts = result.filter(p => /^(0|[1-9][0-9]*)$/.test(p.name) && p.value?.objectId);
      if (hosts.length > 256) throw new Error('MP-11: closed-root candidate bounds');
      const ids = await world.connection.send('Runtime.callFunctionOn', { objectId, functionDeclaration: `function(){return this.map(node=>${world.ref}.idOfNode.call(node))}`, returnByValue: true }, world.sessionId, { timeoutMs: world.patience?.() });
      if (ids.exceptionDetails || !Array.isArray(ids.result?.value) || ids.result.value.length !== hosts.length) throw new Error('MP-11: closed-root candidate identities unavailable');
      // Retain request ordering but let CDP coalesce the narrow descriptions.
      const descriptions = await Promise.all(hosts.map(p => world.connection.send('DOM.describeNode', { objectId: p.value.objectId, depth: 0, pierce: true }, world.sessionId)));
      const inspected = await Promise.allSettled(hosts.map(async (host, i) => {
        const hostId = host.value.objectId;
        try {
          if (descriptions[i].node?.shadowRoots?.some(root => root.shadowRootType === 'closed')) {
            const value = await world.connection.send('Runtime.callFunctionOn', { objectId: hostId, functionDeclaration: `function(fresh){return ${world.ref}.markClosedHost.call(this,fresh)}`, arguments: [{ value: fresh.has(ids.result.value[Number(host.name)]) }], returnByValue: true }, world.sessionId, { timeoutMs: world.patience?.() });
            if (value.exceptionDetails || !value.result?.value) throw new Error('MP-11: closed-root marking unavailable');
            const patch = value.result.value;
            if (!patch.record) published++;
            if (patch.record) { replacements.set(patch.record.id, patch.record); for (const id of patch.removed) removed.add(id); }
          }
          const checked = await world.connection.send('Runtime.callFunctionOn', { objectId: hostId, functionDeclaration: `function(){return ${world.ref}.closedChecked.call(this)}`, returnByValue: true }, world.sessionId, { timeoutMs: world.patience?.() });
          if (checked.exceptionDetails || checked.result?.value !== true) throw new Error('MP-11: closed-root inspection unavailable');
        } finally { await world.connection.send('Runtime.releaseObject', { objectId: hostId }, world.sessionId).catch(() => {}); }
      }));
      const failure = inspected.find(result => result.status === 'rejected');
      if (failure) throw failure.reason;
      if (!hosts.length) break;
    } finally { await world.connection.send('Runtime.releaseObjectGroup', { objectGroup }, world.sessionId).catch(() => {}); }
  }
  const patch = rows => rows.filter(r => !removed.has(r.id)).map(r => replacements.get(r.id) ?? r);
  if (source?.nodes) source.nodes = patch(source.nodes);
  if (source?.ops) source.ops = source.ops.filter(op => !removed.has(op.id) && !replacements.has(op.id)).map(op => op.op === 'children' ? { ...op, nodes: patch(op.nodes), children: op.children.filter(id => !removed.has(id)) } : op);
  if (replacements.size) source.opaque = true;
  return published;
}
