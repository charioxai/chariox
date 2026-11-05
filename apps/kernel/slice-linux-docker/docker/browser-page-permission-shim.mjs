// MP-08/MP-09/MP-10/MP-11 BROWSER09. Main-world API wrappers: CDP has no
// permission-prompt event. Native permission checks still gate the actual API.
// The wrappers must be indistinguishable from the browser's own functions: they live on the
// prototypes where Chromium defines them, keep its names, lengths and descriptors, report native
// source text, and leave no named global behind. Pages (Google sign-in among them) refuse a
// browser whose permission APIs look replaced.
export function installPagePermissionShim(bindingName, responseSlot) {
  const slot = Symbol.for(responseSlot);
  if (Object.hasOwn(globalThis, slot)) return;
  const binding = globalThis[bindingName];
  if (typeof binding !== 'function') return;
  // The binding was only needed to capture the function; no page script can find it by name.
  delete globalThis[bindingName];
  const nativeSource = new WeakMap();
  const define = (target, key, change) => {
    const descriptor = Object.getOwnPropertyDescriptor(target, key);
    if (!descriptor) return null;
    change(descriptor);
    Object.defineProperty(target, key, descriptor);
    return descriptor;
  };
  // A method definition has no "prototype" and cannot be constructed, like Chromium's own functions.
  const asMethod = (name, impl) => ({[name](...args) { return Reflect.apply(impl, this, args); }})[name];
  const named = (fn, name, length, prefix = '') => {
    Object.defineProperty(fn, 'name', {value: prefix + name, configurable: true});
    Object.defineProperty(fn, 'length', {value: length, configurable: true});
    nativeSource.set(fn, `function ${prefix}${name}() { [native code] }`);
    return fn;
  };
  // Replace a data property or accessor the way Chromium defines it, keeping every flag.
  const wrapMethod = (target, key, make) => {
    const original = target?.[key];
    if (typeof original !== 'function') return;
    define(target, key, descriptor => { descriptor.value = named(asMethod(original.name, make(original)), original.name, original.length); });
  };
  const wrapGetter = (target, key, make) => {
    const original = Object.getOwnPropertyDescriptor(target ?? {}, key)?.get;
    if (original) define(target, key, descriptor => { descriptor.get = named(asMethod(key, make(original)), key, 0, 'get '); });
  };
  const toString = Function.prototype.toString;
  const patchedToString = new Proxy(toString, {apply: (target, self, args) => nativeSource.get(self) ?? Reflect.apply(target, self, args)});
  nativeSource.set(patchedToString, 'function toString() { [native code] }');
  define(Function.prototype, 'toString', descriptor => { descriptor.value = patchedToString; });

  let serial = 0;
  const pending = new Map(), states = new Map(), statuses = new Map(), statusNames = new WeakMap();
  const blocked = new Set(['camera', 'microphone', 'display-capture']);
  const covered = new Set(['geolocation', 'notifications', 'clipboard-read', ...blocked]);
  const state = name => blocked.has(name) ? 'denied' : states.get(name) ?? 'prompt';
  const update = (name, value) => {
    const old = state(name); states.set(name, value);
    if (old !== value) for (const ref of statuses.get(name) ?? []) {
      const status = ref.deref();
      if (status) status.dispatchEvent(new Event('change'));
      else statuses.get(name).delete(ref);
    }
  };
  Object.defineProperty(globalThis, slot, {value:(id, value, permission) => {
    if (id === null && covered.has(permission)) { update(permission, value); return; }
    const item = pending.get(id); if (!item) return;
    pending.delete(id); clearTimeout(item.timer); update(item.permission, value); item.resolve(value);
  }});
  const ask = (permission, op = 'request') => new Promise(resolve => {
    if (blocked.has(permission)) { resolve('denied'); return; }
    const id = String(++serial);
    // An unanswered query leaves the state alone (a page's first query can precede the kernel's document
    // registration); only an unanswered request is a refusal.
    const timer = setTimeout(() => { pending.delete(id); if (op === 'query') { resolve(state(permission)); return; } update(permission, 'denied'); resolve('denied'); }, op === 'query' ? 5000 : 31000);
    pending.set(id, {resolve, timer, permission});
    try { binding(JSON.stringify({id, permission, op})); }
    catch { globalThis[slot](id, 'denied'); }
  });
  const denied = () => new DOMException('Permission denied', 'NotAllowedError');
  const unavailable = () => new DOMException('Camera, microphone and screen capture are unavailable in this browser', 'NotAllowedError');
  const rejected = error => Promise.reject(error);

  if (globalThis.Permissions) {
    wrapMethod(Permissions.prototype, 'query', query => function (descriptor) {
      if (!covered.has(descriptor?.name)) return Reflect.apply(query, this, [descriptor]);
      return Reflect.apply(query, this, [descriptor]).then(async status => {
        update(descriptor.name, await ask(descriptor.name, 'query'));
        statusNames.set(status, descriptor.name);
        const list = statuses.get(descriptor.name) ?? new Set(); list.add(new WeakRef(status)); statuses.set(descriptor.name, list);
        return status;
      });
    });
    wrapGetter(globalThis.PermissionStatus?.prototype, 'state', getState => function () {
      const name = statusNames.get(this);
      return name === undefined ? Reflect.apply(getState, this, []) : state(name);
    });
  }
  if (globalThis.Geolocation) {
    const proto = Geolocation.prototype;
    const watches = new Map(); let watchId = 0;
    const refuse = error => error?.({code:1, PERMISSION_DENIED:1, POSITION_UNAVAILABLE:2, TIMEOUT:3, message:'Geolocation permission denied'});
    wrapMethod(proto, 'getCurrentPosition', get => function (success, error, options) {
      void ask('geolocation').then(value => value === 'granted' ? Reflect.apply(get, this, [success, error, options]) : refuse(error));
    });
    wrapMethod(proto, 'watchPosition', watch => function (success, error, options) {
      const id = ++watchId; watches.set(id, null);
      void ask('geolocation').then(value => {
        if (!watches.has(id)) return;
        if (value === 'granted') watches.set(id, Reflect.apply(watch, this, [success, error, options]));
        else { watches.delete(id); refuse(error); }
      }); return id;
    });
    wrapMethod(proto, 'clearWatch', clear => function (id) {
      const native = watches.get(id); watches.delete(id);
      if (native != null) Reflect.apply(clear, this, [native]);
    });
  }
  if (globalThis.Notification) {
    const nativePermission = Object.getOwnPropertyDescriptor(Notification, 'permission')?.get;
    wrapGetter(Notification, 'permission', () => function () {
      return states.has('notifications') ? state('notifications') === 'prompt' ? 'default' : state('notifications') : nativePermission?.call(Notification) === 'granted' ? 'granted' : 'default';
    });
    void ask('notifications', 'query');
    wrapMethod(Notification, 'requestPermission', () => function (callback) {
      return ask('notifications').then(value => { callback?.(value); return value; });
    });
  }
  if (globalThis.Clipboard) for (const name of ['read', 'readText']) {
    wrapMethod(Clipboard.prototype, name, read => function (...args) {
      return ask('clipboard-read').then(value => value === 'granted' ? Reflect.apply(read, this, args) : rejected(denied()));
    });
  }
  if (globalThis.MediaDevices) for (const name of ['getUserMedia', 'getDisplayMedia']) {
    wrapMethod(MediaDevices.prototype, name, () => function () { return rejected(unavailable()); });
  }
  for (const name of ['getUserMedia', 'webkitGetUserMedia']) {
    wrapMethod(Navigator.prototype, name, () => function (_constraints, _success, error) { queueMicrotask(() => error?.(unavailable())); });
  }
}
