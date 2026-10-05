// MP-08/MP-10/MP-11: bounded native input on the same resolved Browser target.
export const BROWSER_KEYS = Object.freeze({ArrowLeft:37, ArrowUp:38, ArrowRight:39, ArrowDown:40,
  Home:36, End:35, PageUp:33, PageDown:34, Enter:13, Escape:27, Tab:9, Space:32, Backspace:8, Delete:46});
export class BrowserInteractionError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
const invalid = message => new BrowserInteractionError('browser_action_invalid',message);
const cancelled = signal => { if (signal?.aborted) throw new BrowserInteractionError('browser_action_cancelled','browser action was cancelled'); };

export function normalizeBrowserInteraction(action) {
  if (!['press','drag'].includes(action?.kind)) return null;
  const expected = action.expected_value ?? null;
  if (expected !== null && (typeof expected !== 'string' || expected.length > 64 ||
    !/^-?(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(expected) || !Number.isFinite(Number(expected))))
    throw invalid('expected range value must be a finite numeric string');
  if (action.kind === 'press') {
    if (!Object.hasOwn(BROWSER_KEYS, action.key)) throw invalid('unsupported target-bound browser key');
    return {kind:'press', key:action.key, expected_value:expected};
  }
  if (![action.delta_x,action.delta_y].every(n => Number.isSafeInteger(n) && Math.abs(n) <= 2048) ||
    action.delta_x === 0 && action.delta_y === 0) throw invalid('drag needs nonzero integer deltas within 2048 CSS pixels');
  return {kind:'drag',delta_x:action.delta_x,delta_y:action.delta_y,expected_value:expected};
}

async function inspect(connection, sessionId, objectId, expected, focus) {
  const response = await connection.send('Runtime.callFunctionOn', {objectId,
    functionDeclaration: inspectInteractionTarget.toString(),
    arguments:[{value:expected},{value:focus}], returnByValue:true}, sessionId);
  if (response?.exceptionDetails || response?.result?.value?.ok !== true)
    throw new BrowserInteractionError(focus ? 'browser_interaction_not_focusable' : 'browser_interaction_not_applied',
      'browser interaction target or expected range value was not verified');
}

export function inspectInteractionTarget(expected, focus, verify = true) {
  if (!this.isConnected || this.matches?.(':disabled') || this.closest?.('[inert]') ||
    this.getAttribute?.('aria-disabled') === 'true') return {ok:false};
  const native = this.tagName === 'INPUT' && this.type === 'range';
  const aria = this.getAttribute?.('role') === 'slider';
  if ((native || aria) && (this.readOnly || this.getAttribute?.('aria-readonly') === 'true')) return {ok:false};
  // Verify numeric slider values only; never read an editable/password value.
  if (expected !== null && !(native || aria)) return {ok:false};
  if (focus) {
    this.focus({preventScroll:true});
    return {ok:this.getRootNode().activeElement === this};
  }
  if (!verify) return {ok:true};
  const ariaValue = aria ? this.getAttribute('aria-valuenow') : null;
  const value = native ? this.valueAsNumber : ariaValue !== null && ariaValue.trim() !== '' ? Number(ariaValue) : null;
  return {ok:expected === null || Number.isFinite(value) && value === Number(expected)};
}

export async function executeBrowserInteraction({connection, sessionId, objectId, geometry, action, signal}) {
  cancelled(signal);
  if (action.kind === 'press') {
    // Checking the expected value's supported control happens before input.
    await inspect(connection, sessionId, objectId, action.expected_value, true);
    const key = action.key === 'Space' ? ' ' : action.key;
    const params = {key,code:action.key,windowsVirtualKeyCode:BROWSER_KEYS[action.key],nativeVirtualKeyCode:BROWSER_KEYS[action.key]};
    const text = action.key === 'Enter' ? '\r' : action.key === 'Space' ? ' ' : null;
    let down = false;
    try {
      cancelled(signal); down = true;
      await connection.send('Input.dispatchKeyEvent',{...params,type:'keyDown',
        ...(text === null ? {} : {text,unmodifiedText:text})},sessionId);
    } finally {
      if (down) await connection.send('Input.dispatchKeyEvent',{...params,type:'keyUp'},sessionId);
    }
  } else {
    const {cssVisualViewport: viewport} = await connection.send('Page.getLayoutMetrics',{},sessionId);
    const x = geometry.x + action.delta_x, y = geometry.y + action.delta_y;
    if (!viewport || ![x,y,viewport.clientWidth,viewport.clientHeight].every(Number.isFinite) ||
      x < 0 || y < 0 || x >= viewport.clientWidth || y >= viewport.clientHeight)
      throw invalid('drag destination is outside the current Browser viewport');
    if (action.expected_value !== null) {
      // Preflight validates the control without focusing or checking the old value.
      const result = await connection.send('Runtime.callFunctionOn',{objectId,
        functionDeclaration:inspectInteractionTarget.toString(),
        arguments:[{value:action.expected_value},{value:false},{value:false}],
        returnByValue:true},sessionId);
      if (result?.exceptionDetails || result?.result?.value?.ok !== true) throw invalid('expected value requires an observed numeric slider');
    }
    let pressed = false, releaseX = geometry.x, releaseY = geometry.y;
    try {
      cancelled(signal);
      await connection.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:geometry.x,y:geometry.y},sessionId);
      cancelled(signal); pressed = true;
      await connection.send('Input.dispatchMouseEvent',{type:'mousePressed',x:geometry.x,y:geometry.y,button:'left',buttons:1,clickCount:1},sessionId);
      for (let i=1;i<=8;i++) {
        cancelled(signal);
        releaseX = geometry.x+action.delta_x*i/8;
        releaseY = geometry.y+action.delta_y*i/8;
        await connection.send('Input.dispatchMouseEvent',{type:'mouseMoved',x:releaseX,
          y:releaseY,button:'left',buttons:1},sessionId);
      }
    } finally {
      // Release even after cancellation or an ambiguous acknowledgement; never
      // replay the drag. This cleanup input cannot start another interaction.
      if (pressed) await connection.send('Input.dispatchMouseEvent',{type:'mouseReleased',x:releaseX,y:releaseY,button:'left',buttons:0,clickCount:1},sessionId);
    }
  }
  cancelled(signal);
  await inspect(connection,sessionId,objectId,action.expected_value,false);
  return {dialogOpened:false};
}
