// MP-08/MP-10/MP-11: one trusted isolated world per live CDP document.
export const FOCUS_WORLD = 'chariox-controller-focus';
export async function ensureFocusWorld(browser, connection, sessionId, targetId, frame) {
  const cache = browser.focusWorldsByTarget;
  let world = cache.get(targetId);
  if (!world || world.documentId !== frame.loaderId || world.frameId !== frame.id || world.connection !== connection || world.sessionId !== sessionId) {
    world = { documentId: frame.loaderId, frameId: frame.id, connection, sessionId };
    cache.set(targetId, world);
    const creating = world;
    creating.ready = (async () => {
      try {
        const result = await connection.send('Page.createIsolatedWorld', {
          frameId: frame.id, worldName: FOCUS_WORLD, grantUniveralAccess: false,
        }, sessionId);
        creating.contextId = result?.executionContextId;
        return creating;
      } catch (error) {
        if (cache.get(targetId) === creating) cache.delete(targetId);
        throw error;
      } finally { delete creating.ready; }
    })();
  }
  return world.ready ?? world;
}
