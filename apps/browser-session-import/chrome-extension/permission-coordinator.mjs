const permissionError = () => Object.assign(new Error('cookie_source_denied'),{code:'cookie_source_denied'});
const storageKey = 'browser_import_permission_leases_v1';
const maximumOperations = 128;
const maximumPermissionValues = 1 + maximumOperations * 32;
const maximumLateAcquisitionMs = 120_000;

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

export function createChromeSessionPermissionStateStore(chrome) {
  if (typeof chrome?.storage?.session?.get !== 'function'
      || typeof chrome?.storage?.session?.set !== 'function') throw permissionError();
  return Object.freeze({
    async load() {
      const stored = await chrome.storage.session.get(storageKey);
      return stored?.[storageKey];
    },
    async save(value) { await chrome.storage.session.set({[storageKey]:value}); },
  });
}

export async function isLiveConnectorDocument(chrome,owner,connectorUrl) {
  try {
    const tabId=validateOwner(owner?.tabId);
    const documentId=validateDocument(owner?.documentId);
    if (tabId === null || documentId === null || typeof chrome?.runtime?.getContexts !== 'function') return false;
    const connector=new URL(connectorUrl);
    const contexts=await chrome.runtime.getContexts({contextTypes:['TAB'],tabIds:[tabId]});
    return Array.isArray(contexts) && contexts.some(context => {
      if (context?.tabId !== tabId || context?.documentId !== documentId) return false;
      const current=new URL(context.documentUrl);
      return current.origin === connector.origin && current.pathname === connector.pathname;
    });
  } catch { return false; }
}

export class PermissionGrantCoordinator {
  #stateStore;
  #queue = Promise.resolve();
  constructor(remove,{stateStore = memoryStateStore(),now = () => Date.now()} = {}) {
    if (typeof remove !== 'function' || typeof stateStore?.load !== 'function'
        || typeof stateStore?.save !== 'function' || typeof now !== 'function') throw permissionError();
    this.remove = remove;
    this.#stateStore = stateStore;
    this.now = now;
  }
  reserve(id,permission,preexisting,{ownerTabId = null,ownerDocumentId = null} = {}) {
    return this.#run(async state => {
      const operationId = validateOperationId(id);
      const value = validatePermission(permission);
      const prior = validateSnapshot(value,preexisting);
      const owner = validateOwner(ownerTabId);
      const ownerDocument = validateDocument(ownerDocumentId);
      const existing = state.operations[operationId];
      const preexistingValues = [];
      if (prior.cookies) preexistingValues.push('cookies');
      for (const origin of value.origins) if (prior.origins[origin]) preexistingValues.push(origin);
      if (existing) {
        const expectedAcquired = ['cookies',...value.origins].filter(entry => !preexistingValues.includes(entry));
        if (existing.owner_tab_id !== owner || existing.owner_document_id !== ownerDocument
            || !same(existing.needs,['cookies',...value.origins])
            || !same(existing.preexisting,preexistingValues)
            || !same(existing.acquired,expectedAcquired)) {
          throw permissionError();
        }
        return;
      }
      if (Object.keys(state.operations).length >= maximumOperations) throw permissionError();
      const acquired = [];
      if (!prior.cookies) acquired.push('cookies');
      for (const origin of value.origins) if (!prior.origins[origin]) acquired.push(origin);
      state.operations[operationId] = {owner_tab_id:owner,owner_document_id:ownerDocument,
        needs:['cookies',...value.origins],
        preexisting:preexistingValues,
        acquired:[...acquired],pending:[...acquired],observed_added:[],active:false,updated_at_ms:this.now()};
      await this.#stateStore.save(state);
    });
  }
  activate(id,ownerTabId = undefined,ownerDocumentId = undefined) {
    return this.#run(async state => {
      const operation = ownedOperation(state,id,ownerTabId,ownerDocumentId);
      if (operation.active) return;
      operation.active = true;
      operation.updated_at_ms = this.now();
      for (const value of operation.acquired) addUnique(state.transient_owned,value);
      operation.pending = [];
      operation.observed_added = [];
      await this.#stateStore.save(state);
    });
  }
  observeAdded(permission) {
    return this.#run(async state => {
      const added = permissionValues(permission);
      purgeLateAcquisitions(state,this.now());
      for (const operation of Object.values(state.operations)) {
        for (const value of operation.pending) if (added.has(value)) addUnique(operation.observed_added,value);
        operation.pending = operation.pending.filter(value => !added.has(value));
      }
      for (const entry of state.late_acquisitions) if (added.has(entry.value)) {
        addUnique(state.transient_owned,entry.value);
      }
      state.late_acquisitions = state.late_acquisitions.filter(entry => !added.has(entry.value));
      await this.#stateStore.save(state);
      await this.#removeUnused(state);
    });
  }
  release(id,ownerTabId = undefined,{outcome = 'terminal',ownerDocumentId = undefined} = {}) {
    return this.#run(async state => {
      const operationId = validateOperationId(id);
      const operation = state.operations[operationId];
      validateOutcome(outcome);
      if (operation) assertOwner(operation,ownerTabId,ownerDocumentId);
      if (operation) {
        delete state.operations[operationId];
        if (outcome === 'granted') for (const value of operation.acquired) {
          addUnique(state.transient_owned,value);
        }
        if (outcome === 'possible_late') {
          for (const value of operation.observed_added) addUnique(state.transient_owned,value);
          for (const value of operation.pending) addLateAcquisition(state,{operationId,value,
            expiresAtMs:this.now() + maximumLateAcquisitionMs});
        }
        await this.#stateStore.save(state);
      }
      await this.#removeUnused(state);
    });
  }
  releaseOwner(ownerTabId) {
    return this.#run(async state => {
      const owner = validateOwner(ownerTabId);
      for (const [id,operation] of Object.entries(state.operations)) if (operation.owner_tab_id === owner) {
        delete state.operations[id];
        for (const value of operation.observed_added) addUnique(state.transient_owned,value);
        for (const value of operation.pending) addLateAcquisition(state,{operationId:id,value,
          expiresAtMs:this.now() + maximumLateAcquisitionMs});
      }
      await this.#stateStore.save(state);
      await this.#removeUnused(state);
    });
  }
  recover({ownerAlive = async () => true} = {}) {
    if (typeof ownerAlive !== 'function') return Promise.reject(permissionError());
    return this.#run(async state => {
      purgeLateAcquisitions(state,this.now());
      for (const [id,operation] of Object.entries(state.operations)) {
        if (operation.owner_tab_id === null || await ownerAlive({tabId:operation.owner_tab_id,
          documentId:operation.owner_document_id})) continue;
        delete state.operations[id];
        for (const value of operation.observed_added) addUnique(state.transient_owned,value);
        for (const value of operation.pending) addLateAcquisition(state,{operationId:id,value,
          expiresAtMs:this.now() + maximumLateAcquisitionMs});
      }
      await this.#stateStore.save(state);
      await this.#removeUnused(state);
    });
  }
  #run(action) {
    const run = async () => action(validateState(upgradeState(await this.#stateStore.load(),this.now())));
    this.#queue = this.#queue.then(run,run);
    return this.#queue;
  }
  async #removeUnused(state) {
    const needed = new Set();
    for (const operation of Object.values(state.operations)) for (const value of operation.needs) needed.add(value);
    for (const value of [...state.transient_owned]) {
      if (needed.has(value)) continue;
      const details = value === 'cookies' ? {permissions:['cookies']} : {origins:[value]};
      try {
        if (await this.remove(details) === true) {
          state.transient_owned = state.transient_owned.filter(entry => entry !== value);
          await this.#stateStore.save(state);
        }
      } catch { /* retain ownership metadata for a later alarm or terminal cleanup */ }
    }
  }
}

export function createChromePermissionLifecycle(chrome,{timeoutMs = 5000,
  operationId = crypto.randomUUID()} = {}) {
  const fixedOperationId = validateOperationId(operationId);
  let port;
  let reserved = false;
  let released = false;
  let releasePromise;
  let sequence = 0;
  const pending = new Map();
  const disconnect = () => {
    port = undefined;
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
  const ensurePort = () => {
    if (port) return port;
    if (released || typeof chrome?.runtime?.connect !== 'function') throw permissionError();
    port = chrome.runtime.connect({name:'browser-import-permission-lease'});
    port.onMessage.addListener(message);
    port.onDisconnect.addListener(disconnect);
    return port;
  };
  const rpcOnce = (kind,payload = {}) => new Promise((resolve,reject) => {
    let connected;
    try { connected = ensurePort(); } catch { reject(permissionError()); return; }
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(permissionError()); },timeoutMs);
    pending.set(id,{resolve,reject,timer});
    try { connected.postMessage({kind,id,operation_id:fixedOperationId,...payload}); }
    catch { pending.delete(id); clearTimeout(timer); port = undefined; reject(permissionError()); }
  });
  const rpc = async (kind,payload = {}) => {
    try { await rpcOnce(kind,payload); }
    catch {
      if (released) throw permissionError();
      await rpcOnce(kind,payload);
    }
  };
  return Object.freeze({
    async reserve(permission) {
      if (reserved || released) throw permissionError();
      const validated = validatePermission(permission);
      const preexisting = await snapshotPermissionState(chrome,validated);
      await rpc('reserve',{permission:validated,preexisting});
      reserved = true;
    },
    async activate() {
      if (!reserved || released) throw permissionError();
      await rpc('activate');
    },
    async release({outcome = 'terminal'} = {}) {
      validateOutcome(outcome);
      if (releasePromise) return releasePromise;
      releasePromise = (async () => {
        if (!reserved) { released = true; return; }
        try { await rpc('release',{outcome}); }
        finally { released = true; try { port?.disconnect(); } catch { /* already disconnected */ } }
      })();
      return releasePromise;
    },
  });
}

function memoryStateStore() {
  let state;
  return {async load() { return state; },async save(value) { state = structuredClone(value); }};
}
function emptyState() { return {version:2,operations:{},late_acquisitions:[],transient_owned:[]}; }
function upgradeState(value,now) {
  if (value === undefined || value?.version !== 1) return value;
  if (!plain(value) || !exactKeys(value,['version','operations','late_acquisitions','transient_owned'])
      || !plain(value.operations) || Object.keys(value.operations).length > maximumOperations
      || !Array.isArray(value.late_acquisitions) || !Array.isArray(value.transient_owned)) throw permissionError();
  validateValueSet(value.late_acquisitions,maximumPermissionValues);
  validateValueSet(value.transient_owned,maximumPermissionValues);
  const upgraded=emptyState();
  upgraded.transient_owned=[...value.transient_owned];
  let legacy=0;
  for (const [id,operation] of Object.entries(value.operations)) {
    validateOperationId(id);
    if (!plain(operation) || !exactKeys(operation,['owner_tab_id','needs','preexisting','acquired','pending',
      'active','updated_at_ms']) || !Array.isArray(operation.needs) || !Array.isArray(operation.preexisting)
      || !Array.isArray(operation.acquired) || !Array.isArray(operation.pending)
      || typeof operation.active !== 'boolean' || !Number.isSafeInteger(operation.updated_at_ms)
      || operation.updated_at_ms < 0) throw permissionError();
    validateOwner(operation.owner_tab_id);
    for (const list of [operation.needs,operation.preexisting,operation.acquired,operation.pending]) {
      validateValueSet(list,33);
    }
    if (operation.needs[0] !== 'cookies'
        || operation.preexisting.some(entry => !operation.needs.includes(entry))
        || operation.acquired.some(entry => !operation.needs.includes(entry)
          || operation.preexisting.includes(entry))
        || operation.pending.some(entry => !operation.acquired.includes(entry))) throw permissionError();
    const expiresAtMs=operation.updated_at_ms + maximumLateAcquisitionMs;
    if (expiresAtMs > now) for (const entry of operation.pending) {
      addLateAcquisition(upgraded,{operationId:id,value:entry,expiresAtMs});
    }
  }
  for (const entry of value.late_acquisitions) addLateAcquisition(upgraded,
    {operationId:`legacy-${legacy++}`,value:entry,expiresAtMs:now + maximumLateAcquisitionMs});
  return upgraded;
}
function validateState(value) {
  if (value === undefined) return emptyState();
  if (!plain(value) || !exactKeys(value,['version','operations','late_acquisitions','transient_owned'])
      || value.version !== 2 || !plain(value.operations)
      || Object.keys(value.operations).length > maximumOperations
      || !Array.isArray(value.late_acquisitions) || !Array.isArray(value.transient_owned)) throw permissionError();
  if (value.late_acquisitions.length > maximumPermissionValues) throw permissionError();
  for (const entry of value.late_acquisitions) {
    if (!plain(entry) || !exactKeys(entry,['operation_id','value','expires_at_ms'])
        || validateOperationId(entry.operation_id) !== entry.operation_id
        || validatePermissionValue(entry.value) !== entry.value
        || !Number.isSafeInteger(entry.expires_at_ms) || entry.expires_at_ms < 0) throw permissionError();
  }
  validateValueSet(value.transient_owned,maximumPermissionValues);
  for (const [id,operation] of Object.entries(value.operations)) {
    validateOperationId(id);
    if (!plain(operation) || !exactKeys(operation,['owner_tab_id','owner_document_id','needs','preexisting',
      'acquired','pending','observed_added','active','updated_at_ms'])
        || !Array.isArray(operation.needs) || !Array.isArray(operation.preexisting)
        || !Array.isArray(operation.acquired)
        || !Array.isArray(operation.pending) || !Array.isArray(operation.observed_added)
        || typeof operation.active !== 'boolean'
        || !Number.isSafeInteger(operation.updated_at_ms) || operation.updated_at_ms < 0) throw permissionError();
    validateOwner(operation.owner_tab_id);
    validateDocument(operation.owner_document_id);
    for (const list of [operation.needs,operation.preexisting,operation.acquired,operation.pending,
      operation.observed_added]) {
      validateValueSet(list,33);
    }
    if (operation.needs[0] !== 'cookies'
        || operation.preexisting.some(value => !operation.needs.includes(value))
        || operation.acquired.some(value => !operation.needs.includes(value)
          || operation.preexisting.includes(value))
        || operation.pending.some(value => !operation.acquired.includes(value))
        || operation.observed_added.some(value => !operation.acquired.includes(value))) throw permissionError();
  }
  return value;
}
function ownedOperation(state,id,ownerTabId,ownerDocumentId) {
  const operation = state.operations[validateOperationId(id)];
  if (!operation) throw permissionError();
  assertOwner(operation,ownerTabId,ownerDocumentId);
  return operation;
}
function assertOwner(operation,ownerTabId,ownerDocumentId) {
  if ((ownerTabId !== undefined && operation.owner_tab_id !== validateOwner(ownerTabId))
      || (ownerDocumentId !== undefined
        && operation.owner_document_id !== validateDocument(ownerDocumentId))) throw permissionError();
}
function permissionValues(permission) {
  const added = new Set();
  if (Array.isArray(permission?.permissions) && permission.permissions.includes('cookies')) added.add('cookies');
  if (Array.isArray(permission?.origins)) for (const origin of permission.origins) {
    try { added.add(validatePermissionValue(origin)); } catch { /* unrelated permission */ }
  }
  return added;
}
function validatePermission(value) {
  if (!plain(value) || Object.keys(value).length !== 2 || !Array.isArray(value.permissions)
      || value.permissions.length !== 1 || value.permissions[0] !== 'cookies'
      || !Array.isArray(value.origins) || value.origins.length < 1 || value.origins.length > 32
      || new Set(value.origins).size !== value.origins.length) throw permissionError();
  value.origins.forEach(validatePermissionValue);
  return Object.freeze({permissions:Object.freeze(['cookies']),origins:Object.freeze([...value.origins])});
}
function validateSnapshot(permission,value) {
  if (!plain(value) || typeof value.cookies !== 'boolean' || !plain(value.origins)
      || Object.keys(value.origins).length !== permission.origins.length
      || permission.origins.some(origin => typeof value.origins[origin] !== 'boolean')) throw permissionError();
  return value;
}
function validatePermissionValue(value) {
  if (value !== 'cookies' && (typeof value !== 'string' || !/^\*:\/\/[a-z0-9.-]+\/\*$/.test(value))) {
    throw permissionError();
  }
  return value;
}
function validateOperationId(value) {
  if (typeof value !== 'string' || value.length < 1 || value.length > 128 || !/^[a-zA-Z0-9-]+$/.test(value)) {
    throw permissionError();
  }
  return value;
}
function validateOwner(value) {
  if (value !== null && (!Number.isSafeInteger(value) || value < 0)) throw permissionError();
  return value;
}
function validateDocument(value) {
  if (value !== null && (typeof value !== 'string' || value.length < 1 || value.length > 128
      || /[\x00-\x1f\x7f]/.test(value))) throw permissionError();
  return value;
}
function validateOutcome(value) {
  if (!['denied','possible_late','granted','terminal'].includes(value)) throw permissionError();
  return value;
}
function addLateAcquisition(state,{operationId,value,expiresAtMs}) {
  const existing=state.late_acquisitions.find(entry => entry.operation_id === operationId && entry.value === value);
  if (existing) existing.expires_at_ms=Math.max(existing.expires_at_ms,expiresAtMs);
  else if (state.late_acquisitions.length < maximumPermissionValues) {
    state.late_acquisitions.push({operation_id:operationId,value,expires_at_ms:expiresAtMs});
  }
}
function purgeLateAcquisitions(state,now) {
  state.late_acquisitions=state.late_acquisitions.filter(entry => entry.expires_at_ms > now);
}
function addUnique(values,value) { if (!values.includes(value)) values.push(value); }
function same(left,right) { return left.length === right.length && left.every((value,index) => value === right[index]); }
function validateValueSet(values,maximum) {
  if (values.length > maximum || new Set(values).size !== values.length) throw permissionError();
  values.forEach(validatePermissionValue);
}
function exactKeys(value,keys) {
  const actual = Object.keys(value);
  return actual.length === keys.length && actual.every(key => keys.includes(key));
}
function plain(value) { return typeof value === 'object' && value !== null && !Array.isArray(value); }
