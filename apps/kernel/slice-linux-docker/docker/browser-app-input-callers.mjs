// A CDP input carries a distinct timestamp label. Match that exact trusted
// event, never the time a handler happens to run. The label is a day ahead of
// the wall clock so a person's concurrent physical input cannot match it.
// Self-contained: installed in the App document before its scripts.
export function createInputCallers() {
  const entries = new Map();
  const callers = new WeakMap();
  const edits = new WeakMap();
  let source;
  let textEdit;
  const clearText = () => { if (textEdit) clearTimeout(textEdit.timer); textEdit = undefined; };
  const fingerprint = element => {
    if (typeof element.value === 'string') return [element.value, element.selectionStart, element.selectionEnd, element.selectionDirection];
    const selection = element.ownerDocument.getSelection();
    return [element.innerHTML, selection?.anchorNode, selection?.anchorOffset, selection?.focusNode, selection?.focusOffset];
  };
  const currentText = edit => entries.get(edit.token)?.timestamp != null && !edit.expired
    && edit.element.isConnected && document.activeElement === edit.element
    && fingerprint(edit.element).every((value, index) => value === edit.fingerprint[index]);
  globalThis.addEventListener('beforeinput', event => {
    const edit = textEdit;
    if (!event.isTrusted || !edit || !['inserting', 'accepted'].includes(edit.state) || event.inputType !== 'insertText' || event.data !== edit.text) return;
    // A physical key's own default edit has its own source, even when its
    // text equals the queued CDP text. It must not consume that insertion.
    if (source?.type === 'keydown' && !source.defaultPrevented && source.target === event.target && /^[^\p{C}]$/u.test(source.key) && source.key === event.data) return;
    // This check runs in the native insertion task, before App listeners and
    // the edit itself. A change after host admission must still type nothing.
    if (edit.state === 'accepted' || edit.element !== event.target || !currentText(edit)) {
      edit.state = 'refused';
      event.preventDefault();
      event.stopImmediatePropagation();
      return;
    }
    edit.state = 'accepted';
    callers.set(event, edit.token);
    edit.before = event;
    clearTimeout(edit.timer);
    // Native input and synchronous editor transforms share this task. A
    // cancelled beforeinput must not lend its caller to a later human input.
    edit.timer = setTimeout(() => { edit.before = undefined; }, 0);
  }, true);
  globalThis.addEventListener('input', event => {
    const edit = textEdit;
    const key = source?.type === 'keydown' && !source.defaultPrevented && source.target === event.target ? source : null;
    const token = edits.get(event.target) ?? (edit?.before && edit.element === event.target ? edit.token : key && callers.get(key));
    if (event.isTrusted && entries.get(token)?.timestamp != null) callers.set(event, token);
    if (event.isTrusted && edit?.before && edit.element === event.target) {
      clearTimeout(edit.timer);
      edit.before = undefined;
    }
    if (key) source = undefined;
  }, true);
  globalThis.addEventListener('submit', event => {
    const origin = source;
    source = undefined;
    if (!origin || origin.defaultPrevented || !event.isTrusted) return;
    const control = origin.type === 'click' ? origin.target.closest?.('button,input') : origin.target;
    if (control?.form !== event.target) return;
    if (origin.type === 'click' ? control.type !== 'submit' || event.submitter !== control
      : origin.key !== 'Enter' || control.tagName !== 'INPUT') return;
    const token = callers.get(origin);
    if (entries.get(token)?.timestamp != null) callers.set(event, token);
  }, true);
  // The label is transport metadata. Capture it before App listeners and
  // expose the actual dispatch time so App debounce/animation clocks stay sane.
  const label = event => Math.round((performance.timeOrigin + event.timeStamp) / 1000);
  for (const type of ['pointerdown','pointerup','pointermove','mousedown','mouseup','mousemove','click','dblclick','keydown','keypress','keyup','wheel']) {
    globalThis.addEventListener(type, event => {
      if (!event.isTrusted) return;
      // The native default action follows propagation. Keep its exact source
      // until its generated input/submit or the next physical event, including
      // an unclaimed human event. There is no wall-clock attribution window.
      const previous = source;
      source = type === 'keydown' ? event : type === 'keypress' && previous?.type === 'keydown' ? previous : undefined;
      // Enter's native implicit submission clicks the default button without
      // the CDP timestamp. It is still part of that key's default action.
      if (type === 'click' && event.detail === 0 && previous?.type === 'keydown'
        && previous.key === 'Enter' && !previous.defaultPrevented
        && previous.target.form && event.target.form === previous.target.form) {
        const token = callers.get(previous);
        if (entries.get(token)?.timestamp != null) { callers.set(event, token); source = event; }
      }
      for (const [token, entry] of entries) if (entry.timestamp === label(event)) {
        callers.set(event, token);
        if (type === 'keydown' || type === 'click') source = event;
        else if (type === 'keypress' && callers.get(previous) === token) source = previous;
        Object.defineProperty(event, 'timeStamp', { value: performance.now() });
        break;
      }
    }, true);
  }
  return Object.freeze({
    begin(token, timestamp) { entries.set(token, { timestamp, handlers: new Set() }); },
    armText(text, token) {
      if (entries.get(token)?.timestamp == null) throw new Error('App input caller expired');
      clearText();
      // Arming records intent, not an event caller. Human input before the
      // dispatch guard remains unscoped, even if it inserts the same text.
      const element = document.activeElement;
      const edit = textEdit = { element, fingerprint: fingerprint(element), text, token, state: 'armed' };
      edit.timer = setTimeout(() => {
        // An in-flight native command still needs a refusal fence until its
        // acknowledgement; expiry removes attribution, never that fence.
        if (edit.state === 'armed') clearText();
        else { edit.expired = true; edit.before = undefined; }
      }, 30_000);
    },
    startText(token) {
      const edit = textEdit;
      if (!edit || edit.token !== token || edit.state !== 'armed' || !currentText(edit)) return false;
      edit.state = 'inserting';
      return true;
    },
    finishText(token) {
      const edit = textEdit;
      return Boolean(edit && edit.token === token && !edit.expired && entries.get(token)?.timestamp != null
        && (edit.state === 'accepted' || edit.state === 'inserting' && currentText(edit)));
    },
    clearText(token) { if (textEdit?.token === token) clearText(); },
    token(event) {
      const token = event && callers.get(event);
      if (entries.get(token)?.timestamp != null) return token;
    },
    submit(form, token) {
      if (entries.get(token)?.timestamp == null) throw new Error('App input caller expired');
      // requestSubmit dispatches synchronously on this exact form. A person's
      // concurrent form or a later event cannot inherit the caller.
      const capture = event => { callers.set(event, token); };
      form.addEventListener('submit', capture, true);
      try { HTMLFormElement.prototype.requestSubmit.call(form); }
      finally { form.removeEventListener('submit', capture, true); }
    },
    edit(element, text, token) {
      if (entries.get(token)?.timestamp == null) throw new Error('App input caller expired');
      // Chromium's native edit emits input synchronously. Capture that exact
      // target only while the command runs, with no caller left on the page.
      edits.set(element, token);
      try {
        if (!element.ownerDocument.execCommand(text ? 'insertText' : 'delete', false, text))
          throw new Error('App text edit failed');
      } finally { edits.delete(element); }
    },
    dispatch(element, event, token) {
      if (entries.get(token)?.timestamp == null) throw new Error('App input caller expired');
      callers.set(event, token);
      return element.dispatchEvent(event);
    },
    track(token, promise) {
      const entry = entries.get(token);
      if (entry) entry.handlers.add(promise);
      // DOM dispatch ignores a listener's returned promise. The controller
      // observes its settlement; a thrown UI handler must not be unhandled.
      promise.catch(() => {});
    },
    async end(token) {
      const entry = entries.get(token);
      if (!entry) return;
      // Dispatch has acknowledged every event. No later event gets this caller;
      // the handlers already holding it retain it until their promises settle.
      entry.timestamp = null;
      if (textEdit?.token === token) {
        if (textEdit.state === 'armed') clearText();
        else { clearTimeout(textEdit.timer); textEdit.expired = true; textEdit.before = undefined; }
      }
      if (source && callers.get(source) === token) source = undefined;
      await Promise.allSettled([...entry.handlers]);
      entries.delete(token);
    },
  });
}
