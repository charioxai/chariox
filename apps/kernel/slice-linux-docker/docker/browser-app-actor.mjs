// MP-08/MP-10/MP-11: native Chromium task ancestry survives timers and async/await.
export const APP_BRIDGE_URL = 'chariox-internal-app-bridge';
export function installAppActorTasks(controlKey, initialTask, ownerTask) {
  const createTask = console.createTask.bind(console), trace = console.trace.bind(console), apply = Reflect.apply;
  const NativeMap = Map, NativeWeakMap = WeakMap;
  const mapGet = Map.prototype.get, mapSet = Map.prototype.set, mapInstance = Function.prototype[Symbol.hasInstance];
  const weakGet = WeakMap.prototype.get, weakSet = WeakMap.prototype.set, setHas = Set.prototype.has;
  const get = (map, key) => apply(apply(mapInstance, NativeMap, [map]) ? mapGet : weakGet, map, [key]);
  const set = (map, key, value) => apply(apply(mapInstance, NativeMap, [map]) ? mapSet : weakSet, map, [key, value]);
  const events = new NativeWeakMap(), listeners = new NativeWeakMap(), properties = new NativeWeakMap();
  const rootInputs = new Set(['click', 'dblclick', 'mousedown', 'mouseup', 'pointerdown', 'pointerup',
    'keydown', 'keyup', 'beforeinput', 'input', 'change', 'wheel', 'submit', 'reset', 'drop', 'contextmenu']);
  let input = ownerTask, dispatching = false, agentSeen = false, executing = null;
  const origin = () => executing ?? (agentSeen ? initialTask + ':unclassified' : initialTask);
  const scoped = (name, task, callback) => apply(task.run, task, [() => {
    const previous = executing; executing = name;
    try { return callback(); } finally { executing = previous; }
  }]);
  const bind = callback => {
    const name = origin(), task = createTask(name);
    return function(...args) { return scoped(name, task, () => apply(callback, this, args)); };
  };
  Object.defineProperty(globalThis, '__charioxAppInput', { value(key, name, active, agent) {
    if (key !== controlKey) return;
    // Between dispatched inputs, trusted events in the window are the owner's own.
    input = active ? name : ownerTask; dispatching = active; if (active) agentSeen ||= agent;
  } });
  const run = (event, callback, receiver) => {
    if (!dispatching && (!apply(setHas, rootInputs, [event.type]) || !event.isTrusted)) return apply(callback, receiver, [event]);
    if (!get(events, event)) set(events, event, input);
    const name = get(events, event);
    // Create at invocation, retaining any outer async origin for JS-caused events.
    return scoped(name, createTask(name), () => apply(callback, receiver, [event]));
  };
  const add = EventTarget.prototype.addEventListener, remove = EventTarget.prototype.removeEventListener;
  EventTarget.prototype.addEventListener = function(type, callback, options) {
    if (!callback) return apply(add, this, [type, callback, options]);
    let callbacks = get(listeners, this);
    if (!callbacks) set(listeners, this, callbacks = new NativeMap());
    let wrapped = get(callbacks, callback);
    if (!wrapped) {
      wrapped = function(event) { return run(event, typeof callback === 'function' ? callback : callback.handleEvent, typeof callback === 'function' ? this : callback); };
      set(callbacks, callback, wrapped);
    }
    return apply(add, this, [type, wrapped, options]);
  };
  EventTarget.prototype.removeEventListener = function(type, callback, options) {
    const callbacks = get(listeners, this);
    return apply(remove, this, [type, callbacks && get(callbacks, callback) || callback, options]);
  };
  for (const prototype of [globalThis, Window.prototype, Document.prototype, HTMLElement.prototype, SVGElement.prototype]) {
    for (const type of rootInputs) {
      const name = 'on' + type, descriptor = Object.getOwnPropertyDescriptor(prototype, name);
      if (!descriptor?.set || !descriptor.configurable) continue;
      const originals = new NativeWeakMap();
      Object.defineProperty(prototype, name, { ...descriptor,
        get() { return get(originals, this) ?? apply(descriptor.get, this, []); },
        set(callback) {
          set(originals, this, callback);
          let marked = get(properties, this);
          if (!marked) set(properties, this, marked = new NativeMap());
          set(marked, name, callback);
          apply(descriptor.set, this, [typeof callback === 'function' ? function(event) { return run(event, callback, this); } : callback]);
        },
      });
    }
  }
  for (const type of rootInputs) apply(add, globalThis, [type, event => {
    for (const target of event.composedPath()) {
      const name = 'on' + type, callback = target[name], marked = get(properties, target);
      if (typeof callback === 'function' && (!marked || get(marked, name) !== callback)) target[name] = callback;
    }
  }, true]);
  // Initial owner refresh/reminder timers keep their origin after agent input.
  // A continuation without a retained origin gets an unclassified tag, never owner.
  for (const name of ['setTimeout', 'setInterval', 'requestAnimationFrame', 'requestIdleCallback', 'queueMicrotask']) {
    const schedule = globalThis[name];
    if (typeof schedule !== 'function') continue;
    globalThis[name] = function(callback, ...args) {
      return apply(schedule, this, [typeof callback === 'function' ? bind(callback) : callback, ...args]);
    };
  }
  const then = Promise.prototype.then;
  Promise.prototype.then = function(ok, error) {
    return apply(then, this, [typeof ok === 'function' ? bind(ok) : ok, typeof error === 'function' ? bind(error) : error]);
  };
  return { trace, bind, run(callback) { const name = origin(); return scoped(name, createTask(name), callback); } };
}

export async function appCallActor(connection, sessionId, app, stack) {
  let origin = { found: false };
  for (let depth = 0; stack && depth < 64; depth++) {
    if (app.actors.has(stack.description) && (stack.description === app.initialTask
      || stack.callFrames.every(frame => frame.scriptId === app.bridgeScript))) {
      // The oldest retained input wins over a nested JS-caused event.
      origin = { found: true, actor: app.actors.get(stack.description) };
    }
    if (stack.parent) stack = stack.parent;
    else if (stack.parentId) stack = (await connection.send('Debugger.getStackTrace', { stackTraceId: stack.parentId }, sessionId)).stackTrace;
    else break;
  }
  return origin;
}
