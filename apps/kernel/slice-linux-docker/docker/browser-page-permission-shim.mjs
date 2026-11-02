// MP-08/MP-09/MP-10/MP-11 BROWSER09. Main-world API wrappers: CDP has no
// permission-prompt event. Native permission checks still gate the actual API.
export function installPagePermissionShim(bindingName, responseName) {
  if (Object.hasOwn(globalThis, responseName)) return;
  const binding = globalThis[bindingName];
  if (typeof binding !== 'function') return;
  let serial = 0;
  const pending = new Map(), states = new Map(), statuses = new Map();
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
  Object.defineProperty(globalThis, responseName, {value:(id, value) => {
    const item = pending.get(id); if (!item) return;
    pending.delete(id); clearTimeout(item.timer); update(item.permission, value); item.resolve(value);
  }});
  const ask = (permission, op = 'request') => new Promise(resolve => {
    if (blocked.has(permission)) { resolve('denied'); return; }
    const id = String(++serial);
    const timer = setTimeout(() => { pending.delete(id); update(permission, 'denied'); resolve('denied'); }, op === 'query' ? 5000 : 31000);
    pending.set(id, {resolve, timer, permission});
    try { binding(JSON.stringify({id, permission, op})); }
    catch { globalThis[responseName](id, 'denied'); }
  });
  const denied = () => new DOMException('Permission denied', 'NotAllowedError');
  const unavailable = () => new DOMException('Camera, microphone and screen capture are unavailable in this browser', 'NotAllowedError');
  if (navigator.permissions) {
    const query = navigator.permissions.query.bind(navigator.permissions);
    navigator.permissions.query = async descriptor => {
      if (!covered.has(descriptor?.name)) return query(descriptor);
      const status = await query(descriptor);
      update(descriptor.name, await ask(descriptor.name, 'query'));
      Object.defineProperty(status, 'state', {get:() => state(descriptor.name)});
      const list = statuses.get(descriptor.name) ?? new Set(); list.add(new WeakRef(status)); statuses.set(descriptor.name, list);
      return status;
    };
  }
  if (navigator.geolocation) {
    const geo = navigator.geolocation;
    const get = geo.getCurrentPosition.bind(geo), watch = geo.watchPosition.bind(geo), clear = geo.clearWatch.bind(geo);
    const watches = new Map(); let watchId = 0;
    const refuse = error => error?.({code:1, PERMISSION_DENIED:1, POSITION_UNAVAILABLE:2, TIMEOUT:3, message:'Geolocation permission denied'});
    geo.getCurrentPosition = (success, error, options) => { void ask('geolocation').then(value => value === 'granted' ? get(success, error, options) : refuse(error)); };
    geo.watchPosition = (success, error, options) => {
      const id = ++watchId; watches.set(id, null);
      void ask('geolocation').then(value => {
        if (!watches.has(id)) return;
        if (value === 'granted') watches.set(id, watch(success, error, options));
        else { watches.delete(id); refuse(error); }
      }); return id;
    };
    geo.clearWatch = id => { const native = watches.get(id); watches.delete(id); if (native != null) clear(native); };
  }
  if (globalThis.Notification) {
    const nativePermission = Object.getOwnPropertyDescriptor(Notification, 'permission')?.get;
    Object.defineProperty(Notification, 'permission', {get:() => states.has('notifications') ? state('notifications') === 'prompt' ? 'default' : state('notifications') : nativePermission?.call(Notification) === 'granted' ? 'granted' : 'default'});
    void ask('notifications', 'query');
    Notification.requestPermission = callback => ask('notifications').then(value => { callback?.(value); return value; });
  }
  if (navigator.clipboard) for (const name of ['read', 'readText']) {
    const read = navigator.clipboard[name]?.bind(navigator.clipboard);
    if (read) navigator.clipboard[name] = async (...args) => { if (await ask('clipboard-read') !== 'granted') throw denied(); return read(...args); };
  }
  if (navigator.mediaDevices) {
    navigator.mediaDevices.getUserMedia = async () => { throw unavailable(); };
    navigator.mediaDevices.getDisplayMedia = async () => { throw unavailable(); };
  }
  for (const name of ['getUserMedia', 'webkitGetUserMedia']) if (navigator[name]) navigator[name] = (_constraints, _success, error) => { queueMicrotask(() => error?.(unavailable())); };
}
