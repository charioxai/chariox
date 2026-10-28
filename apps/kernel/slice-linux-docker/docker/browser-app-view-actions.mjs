// Apps v2 A1: what an App view declares for agents. The page registers actions
// (`chariox.action`, or `chariox.form` with annotated markup), publishes a bounded state
// snapshot (`chariox.view.state`) and announces changes (`chariox.view.changed`).
// The kernel's later A2 bridge reads and drives the page only through
// `__charioxViewAgent`; this module grants no authority, because the signed
// declarations decide what a registration may be (`check_agreement` in
// packages/app-package). Self-contained: it is stringified into the page bridge.
export function createViewActions(request) {
  const NAME = /^[a-z][a-z0-9_]{0,63}$/;
  const EFFECTS = ['read', 'write', 'approval'];
  const FIELDS = new Set(['name', 'description', 'inputSchema', 'outputSchema', 'effect', 'approvalReason']);
  const MAX_STATE_BYTES = 64 * 1024;
  const MAX_RESULT_BYTES = 256 * 1024;
  const MAX_CHANGE_BYTES = 16 * 1024;
  const MAX_BUFFERED_CHANGES = 256;
  const MAX_WAIT_MS = 30_000;
  // Password and protected controls take what the agent supplies like any other:
  // a value the agent knows is its own input. The product masks what a Vault fill
  // put there in model-bound observations; this module neither reads nor echoes it.
  const SKIPPED_INPUTS = /^(?:submit|button|reset|image|file|hidden)$/;

  const registered = new Map();
  const formHandlers = new WeakMap();
  let stateProvider = null;
  let latest = 0;
  const log = [];
  const waiters = new Set();

  const fail = (code, message) => ({ ok: false, code, message: String(message).slice(0, 1024) });
  const failure = error => fail(typeof error?.code === 'string' ? error.code : 'action_failed', error?.message ?? error);
  // JSON-only, size-bounded values: what crosses to the kernel is a copy.
  const bounded = (value, limit, code, what) => {
    let text;
    try { text = JSON.stringify(value ?? null) ?? 'null'; } catch { return fail('not_json', `${what} is not JSON`); }
    const bytes = new TextEncoder().encode(text).length;
    return bytes > limit ? fail(code, `${what} is ${bytes} bytes; the limit is ${limit}`) : { ok: true, value: JSON.parse(text) };
  };

  function describe(declaration) {
    if (!declaration || typeof declaration !== 'object' || Array.isArray(declaration)) throw new TypeError('chariox.action needs a declaration object');
    for (const key of Object.keys(declaration)) if (!FIELDS.has(key)) throw new TypeError(`chariox.action: unknown field "${key}"`);
    if (!NAME.test(declaration.name)) throw new TypeError('chariox.action: name must be lowercase letters, digits and underscores, starting with a letter');
    if (!EFFECTS.includes(declaration.effect)) throw new TypeError(`chariox.action: effect must be one of ${EFFECTS.join(', ')}`);
    if ((declaration.effect === 'approval') !== (declaration.approvalReason !== undefined)) throw new TypeError('chariox.action: approvalReason is required for, and only for, approval actions');
    return JSON.parse(JSON.stringify(declaration));
  }

  function action(declaration, handler) {
    const descriptor = describe(declaration);
    if (typeof handler !== 'function') throw new TypeError('chariox.action needs a handler function');
    if (registered.has(descriptor.name)) throw new Error(`chariox.action: "${descriptor.name}" is already registered`);
    registered.set(descriptor.name, { descriptor, handler });
    // The user path calls the very function an agent runs.
    return input => runHandler(handler, input ?? {});
  }

  async function runHandler(handler, input, token) {
    let active = true;
    const caller = Object.freeze({
      call(method, params = {}) {
        if (!active) return Promise.reject(Object.assign(new Error('The action handler has finished'), { code: 'APP_INVOCATION_EXPIRED' }));
        return request(method, params, token);
      },
    });
    try { return await handler(input, caller); }
    finally { active = false; }
  }

  function form(element, handler) {
    if (element?.tagName !== 'FORM') throw new TypeError('chariox.form needs a form element');
    if (typeof handler !== 'function') throw new TypeError('chariox.form needs a handler function');
    if (element.getAttribute('data-chariox-action') !== null) describe(describeForm(element));
    if (formHandlers.has(element)) throw new Error('chariox.form: this form is already registered');
    formHandlers.set(element, handler);
    element.addEventListener('submit', event => {
      event.preventDefault();
      return runHandler(handler, readForm(element));
    });
  }

  // Forms: only the simple controls map to a schema; anything richer is a
  // `chariox.action`, whose schema the developer writes.
  const controlsOf = form => [...form.elements].filter(control =>
    /^(?:INPUT|SELECT|TEXTAREA)$/.test(control.tagName) && control.name && !control.disabled
    && !(control.tagName === 'INPUT' && SKIPPED_INPUTS.test(control.type))
    && !(control.tagName === 'SELECT' && control.multiple));
  function fieldsOf(form) {
    const fields = new Map();
    for (const control of controlsOf(form)) {
      const known = fields.get(control.name);
      if (known?.radio && control.type === 'radio') { known.schema.enum.push(control.value); known.required ||= control.required; continue; }
      if (known) continue;
      const kind = control.tagName === 'INPUT' ? control.type : control.tagName.toLowerCase();
      const schema = kind === 'checkbox' ? { type: 'boolean' }
        : kind === 'number' || kind === 'range' ? { type: 'number' }
        : kind === 'radio' ? { type: 'string', enum: [control.value] }
        : kind === 'select' ? { type: 'string', enum: [...control.options].filter(option => !option.disabled).map(option => option.value) }
        : { type: 'string' };
      fields.set(control.name, { control, kind, schema, radio: kind === 'radio', required: control.required });
    }
    return fields;
  }
  function describeForm(form) {
    const fields = fieldsOf(form);
    const required = [...fields].filter(([, field]) => field.required).map(([name]) => name);
    const descriptor = {
      name: form.getAttribute('data-chariox-action'),
      description: form.getAttribute('data-chariox-description') ?? '',
      inputSchema: {
        type: 'object',
        properties: Object.fromEntries([...fields].map(([name, field]) => [name, field.schema])),
        ...(required.length ? { required } : {}),
        additionalProperties: false,
      },
      effect: form.getAttribute('data-chariox-effect'),
    };
    const reason = form.getAttribute('data-chariox-approval-reason');
    if (reason !== null) descriptor.approvalReason = reason;
    return descriptor;
  }
  const forms = name => (typeof document === 'undefined' ? [] : [...document.querySelectorAll('form[data-chariox-action]')])
    .filter(form => name === undefined || form.getAttribute('data-chariox-action') === name);

  // Frameworks track a control's last known value and run `onChange` only when
  // the DOM differs from it, so a plain `control.value = x` is swallowed. Text
  // goes through the prototype setter and the events a user's typing fires;
  // a checkbox or radio changes by the native click that fires its events.
  const writeValue = (control, value) => {
    const prototype = control.tagName === 'SELECT' ? HTMLSelectElement.prototype
      : control.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(control, String(value));
    control.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    control.dispatchEvent(new Event('change', { bubbles: true }));
  };

  function readForm(form) {
    const input = {};
    for (const [name, field] of fieldsOf(form)) {
      const control = field.radio ? controlsOf(form).find(c => c.name === name && c.checked) : field.control;
      if (!control) continue;
      input[name] = field.kind === 'checkbox' ? control.checked
        : field.schema.type === 'number' ? Number(control.value) : control.value;
    }
    return input;
  }

  async function submitForm(form, params, token) {
    const handler = formHandlers.get(form);
    if (!handler) return fail('form_unregistered', 'The annotated form must register its handler with chariox.form');
    const fields = fieldsOf(form);
    if (!params || typeof params !== 'object' || Array.isArray(params)) return fail('invalid_params', 'params must be an object');
    const changed = [];
    for (const [name, value] of Object.entries(params)) {
      const field = fields.get(name);
      if (!field) return fail('invalid_params', `the form has no field "${name}"`);
      const expected = field.schema.type === 'boolean' ? 'boolean' : field.schema.type === 'number' ? 'number' : 'string';
      if (typeof value !== expected) return fail('invalid_params', `"${name}" must be a ${expected}`);
      if (field.schema.enum && !field.schema.enum.includes(value)) return fail('invalid_params', `"${name}" must be one of ${field.schema.enum.join(', ')}`);
      changed.push([name, field, value]);
    }
    // Fields the agent leaves out keep what the user already entered.
    for (const [name, field, value] of changed) {
      const control = field.radio ? controlsOf(form).find(c => c.name === name && c.value === value) : field.control;
      if (field.kind === 'checkbox') { if (control.checked !== value) control.click(); }
      else if (field.radio) { if (!control.checked) control.click(); }
      else writeValue(control, value);
    }
    // Let the page's own handlers apply the writes before the form is read.
    await new Promise(resolve => setTimeout(resolve, 0));
    if (!form.checkValidity()) return fail('invalid_form', `the form rejects these fields: ${controlsOf(form).filter(c => !c.checkValidity()).map(c => c.name).join(', ')}`);
    await runHandler(handler, readForm(form), token);
    // Form completion follows the registered handler, including its backend calls.
    return { ok: true, value: { submitted: true } };
  }

  const view = Object.freeze({
    state(provider) {
      if (typeof provider !== 'function') throw new TypeError('chariox.view.state needs a function returning the snapshot');
      stateProvider = provider;
    },
    changed(delta) {
      const checked = bounded(delta, MAX_CHANGE_BYTES, 'change_too_large', 'the change');
      if (!checked.ok) throw new RangeError(`chariox.view.changed: ${checked.message}`);
      log.push({ seq: ++latest, delta: checked.value });
      if (log.length > MAX_BUFFERED_CHANGES) log.shift();
      for (const wake of [...waiters]) wake();
    },
  });

  // The kernel's side. Every answer is `{ok: true, value}` or `{ok: false, code, message}`.
  const agent = Object.freeze({
    /** Declaration-shaped descriptors, compared by the kernel with the signed ones. */
    registrations() {
      return [...[...registered.values()].map(({ descriptor }) => JSON.parse(JSON.stringify(descriptor))), ...forms().map(describeForm)];
    },
    async invoke(name, params, token) {
      const entry = registered.get(name);
      const matching = forms(name);
      if (matching.length + (entry ? 1 : 0) > 1) return fail('duplicate_action', `"${name}" is registered more than once`);
      if (entry) {
        try { return bounded(await runHandler(entry.handler, params ?? {}, token), MAX_RESULT_BYTES, 'result_too_large', 'the result'); } catch (error) { return failure(error); }
      }
      try { return matching.length ? await submitForm(matching[0], params ?? {}, token) : fail('unknown_action', `the view has no action "${name}"`); } catch (error) { return failure(error); }
    },
    async state() {
      if (!stateProvider) return fail('state_unavailable', 'the view did not call chariox.view.state');
      try { return bounded(await stateProvider(), MAX_STATE_BYTES, 'state_too_large', 'the state'); } catch (error) { return failure(error); }
    },
    /** Changes after `since`; with `waitMs`, waits for the first one. `dropped` means some were evicted and the state must be re-read. */
    async changes(since = 0, waitMs = 0) {
      if (latest <= since && waitMs > 0) {
        await new Promise(resolve => {
          const done = () => { clearTimeout(timer); waiters.delete(done); resolve(); };
          const timer = setTimeout(done, Math.min(waitMs, MAX_WAIT_MS));
          waiters.add(done);
        });
      }
      const oldest = log.length ? log[0].seq : latest + 1;
      return { ok: true, value: { latest, dropped: since + 1 < oldest, changes: log.filter(change => change.seq > since) } };
    },
  });

  return { action, form, view, agent };
}
