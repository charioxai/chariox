// A CDP input carries a distinct timestamp label. Match that exact trusted
// event, never the time a handler happens to run. The label is a day ahead of
// the wall clock so a person's concurrent physical input cannot match it.
// Self-contained: installed in the App document before its scripts.
export function createInputCallers() {
  const entries = new Map();
  return Object.freeze({
    begin(token, timestamp) { entries.set(token, { timestamp, handlers: new Set() }); },
    token(event) {
      if (!event?.isTrusted) return undefined;
      const timestamp = Math.round((performance.timeOrigin + event.timeStamp) / 1000);
      for (const [token, entry] of entries) if (entry.timestamp === timestamp) return token;
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
      await Promise.allSettled([...entry.handlers]);
      entries.delete(token);
    },
  });
}
