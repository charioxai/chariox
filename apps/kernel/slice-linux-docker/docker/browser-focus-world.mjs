// MP-08/MP-10/MP-11: one trusted isolated world per live CDP document.
export const FOCUS_WORLD = 'chariox-controller-focus';
export async function ensureFocusWorld(browser, connection, sessionId, targetId, frame, { timeoutMs } = {}) {
  const cache = browser.focusWorldsByTarget;
  let world = cache.get(targetId);
  if (!world || world.documentId !== frame.loaderId || world.frameId !== frame.id || world.connection !== connection || world.sessionId !== sessionId) {
    world = { documentId: frame.loaderId, frameId: frame.id, connection, sessionId };
    cache.set(targetId, world);
    const creating = world;
    const observed = new Promise(resolve => { creating.acceptObservedContext = resolve; });
    const requested = (async () => {
      try {
        const result = await connection.send('Page.createIsolatedWorld', {
          frameId: frame.id, worldName: FOCUS_WORLD, grantUniveralAccess: false,
        }, sessionId, { timeoutMs });
        if (!creating.uniqueContextId) creating.contextId = result?.executionContextId;
        return creating;
      } catch (error) {
        // A trusted creation event is sufficient even if the redundant CDP
        // request times out later. It cannot revive a retired cache entry.
        if (cache.get(targetId) === creating && creating.uniqueContextId) return creating;
        if (cache.get(targetId) === creating) cache.delete(targetId);
        throw error;
      }
    })();
    creating.ready = Promise.race([requested, observed]).finally(() => {
      delete creating.ready; delete creating.acceptObservedContext;
    });
  }
  return world.ready ?? world;
}

// MP-08 / MP-10 / MP-11: consume only the kernel-registered isolated world
// observed after the current main-frame commit. A page's default world never qualifies.
export function recordFocusWorld(browser, message) {
  const sessionId=message?.sessionId,targetId=browser.targetsBySession.get(sessionId);
  if(!targetId)return;
  const cache=browser.focusWorldsByTarget,world=cache.get(targetId);
  const owned=world?.connection===browser.connection&&world.sessionId===sessionId;
  if(message.method==='Runtime.executionContextsCleared') {
    if(owned)cache.delete(targetId);
    return;
  }
  if(message.method==='Runtime.executionContextDestroyed') {
    const params=message.params;
    if(owned&&world.contextId===params?.executionContextId&&
      (!params.executionContextUniqueId||!world.uniqueContextId||params.executionContextUniqueId===world.uniqueContextId))cache.delete(targetId);
    return;
  }
  if(message.method==='Page.frameNavigated'&&!message.params?.frame?.parentId) {
    if(owned&&world.documentId!==message.params?.frame?.loaderId)cache.delete(targetId);
    return;
  }
  if(message.method!=='Runtime.executionContextCreated'||!browser.focusWorldSource)return;
  const context=message.params?.context,frame=browser.currentPageFrame(targetId);
  if(context?.name!==FOCUS_WORLD||context.auxData?.isDefault!==false||context.auxData?.type!=='isolated'||
    !Number.isSafeInteger(context.id)||context.id<=0||typeof context.uniqueId!=='string'||!context.uniqueId.length||context.uniqueId.length>256||
    !frame||frame.connection!==browser.connection||frame.sessionId!==sessionId||frame.frame.id!==context.auxData.frameId||
    typeof frame.frame.loaderId!=='string'||!frame.frame.loaderId)return;
  // Preserve an in-flight creator's object: capture restoration fences also
  // compare its identity. The event adds the unique context retirement fence.
  const current=owned&&world.documentId===frame.frame.loaderId&&world.frameId===frame.frame.id&&
    (!world.uniqueContextId||world.uniqueContextId===context.uniqueId)&&
    (world.contextId===undefined||world.contextId===context.id)?world:
    {connection:browser.connection,sessionId,documentId:frame.frame.loaderId,frameId:frame.frame.id};
  current.contextId=context.id;current.uniqueContextId=context.uniqueId;cache.set(targetId,current);
  current.acceptObservedContext?.(current);
}
