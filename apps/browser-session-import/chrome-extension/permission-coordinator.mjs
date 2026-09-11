const permissionError = () => Object.assign(new Error('cookie_source_denied'),{code:'cookie_source_denied'});

export async function snapshotPermissionState(chrome,permission) {
  const value = validatePermission(permission);
  if (typeof chrome?.permissions?.contains !== 'function') throw permissionError();
  try {
    const cookies = await chrome.permissions.contains({permissions:['cookies']});
    const origins = {};
    for (const origin of value.origins) origins[origin] = await chrome.permissions.contains({origins:[origin]});
    return Object.freeze({cookies:cookies === true,origins:Object.freeze(origins)});
  } catch { throw permissionError(); }
}

export class PermissionGrantCoordinator {
  #operations = new Map();
  #transientOwned = new Set();
  #cleanup = Promise.resolve();
  constructor(remove) {
    if (typeof remove !== 'function') throw permissionError();
    this.remove = remove;
  }
  reserve(id,permission,preexisting) {
    if (this.#operations.has(id)) throw permissionError();
    const value = validatePermission(permission);
    const prior = validateSnapshot(value,preexisting);
    const needs = new Set(['cookies',...value.origins]);
    const acquired = new Set();
    if (!prior.cookies) acquired.add('cookies');
    for (const origin of value.origins) if (!prior.origins[origin]) acquired.add(origin);
    this.#operations.set(id,{needs,acquired,active:false});
  }
  activate(id) {
    const operation = this.#operations.get(id);
    if (!operation || operation.active) throw permissionError();
    operation.active = true;
    for (const value of operation.acquired) this.#transientOwned.add(value);
  }
  observeAdded(permission) {
    const added = new Set();
    if (Array.isArray(permission?.permissions) && permission.permissions.includes('cookies')) added.add('cookies');
    if (Array.isArray(permission?.origins)) for (const origin of permission.origins) {
      if (typeof origin === 'string') added.add(origin);
    }
    for (const operation of this.#operations.values()) {
      for (const value of operation.acquired) if (added.has(value)) this.#transientOwned.add(value);
    }
  }
  release(id) {
    this.#operations.delete(id);
    this.#cleanup = this.#cleanup.then(() => this.#removeUnused(),() => this.#removeUnused());
    return this.#cleanup;
  }
  async #removeUnused() {
    const needed = new Set();
    for (const operation of this.#operations.values()) for (const value of operation.needs) needed.add(value);
    for (const value of [...this.#transientOwned]) {
      if (needed.has(value)) continue;
      const details = value === 'cookies' ? {permissions:['cookies']} : {origins:[value]};
      try {
        if (await this.remove(details) === true) this.#transientOwned.delete(value);
      } catch { /* fixed failure: retain ownership metadata for a later cleanup attempt */ }
    }
  }
}

export function createChromePermissionLifecycle(chrome,{timeoutMs = 5000} = {}) {
  let port;
  let reserved = false;
  let released = false;
  let releasePromise;
  let sequence = 0;
  const pending = new Map();
  const disconnect = () => {
    for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(permissionError()); }
    pending.clear();
  };
  const message = value => {
    const entry = pending.get(value?.id);
    if (!entry) return;
    pending.delete(value.id);
    clearTimeout(entry.timer);
    value.ok === true ? entry.resolve() : entry.reject(permissionError());
  };
  const rpc = (kind,payload = {}) => new Promise((resolve,reject) => {
    if (!port || released) { reject(permissionError()); return; }
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(permissionError()); },timeoutMs);
    pending.set(id,{resolve,reject,timer});
    try { port.postMessage({kind,id,...payload}); }
    catch { pending.delete(id); clearTimeout(timer); reject(permissionError()); }
  });
  return Object.freeze({
    async reserve(permission) {
      if (reserved || released || typeof chrome?.runtime?.connect !== 'function') throw permissionError();
      const validated = validatePermission(permission);
      const preexisting = await snapshotPermissionState(chrome,validated);
      port = chrome.runtime.connect({name:'browser-import-permission-lease'});
      port.onMessage.addListener(message);
      port.onDisconnect.addListener(disconnect);
      await rpc('reserve',{permission:validated,preexisting});
      reserved = true;
    },
    async activate() {
      if (!reserved) throw permissionError();
      await rpc('activate');
    },
    async release() {
      if (releasePromise) return releasePromise;
      releasePromise = (async () => {
        if (!port) { released = true; return; }
        try { await rpc('release'); }
        catch { /* disconnect also releases the background reservation */ }
        finally { released = true; try { port.disconnect(); } catch { /* already disconnected */ } }
      })();
      return releasePromise;
    },
  });
}

function validatePermission(value) {
  if (!plain(value) || Object.keys(value).length !== 2 || !Array.isArray(value.permissions)
      || value.permissions.length !== 1 || value.permissions[0] !== 'cookies'
      || !Array.isArray(value.origins) || value.origins.length < 1 || value.origins.length > 32
      || new Set(value.origins).size !== value.origins.length
      || value.origins.some(origin => typeof origin !== 'string' || !/^\*:\/\/[a-z0-9.-]+\/\*$/.test(origin))) {
    throw permissionError();
  }
  return Object.freeze({permissions:Object.freeze(['cookies']),origins:Object.freeze([...value.origins])});
}
function validateSnapshot(permission,value) {
  if (!plain(value) || typeof value.cookies !== 'boolean' || !plain(value.origins)
      || Object.keys(value.origins).length !== permission.origins.length
      || permission.origins.some(origin => typeof value.origins[origin] !== 'boolean')) throw permissionError();
  return value;
}
function plain(value) { return typeof value === 'object' && value !== null && !Array.isArray(value); }
