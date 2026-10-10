// MP-08/MP-11: creation evidence belongs to one admitted input dispatch, not
// the last actor on its source document. Retain it by the created target so a
// later inventory can adopt the popup without attributing subsequent native input.
export class BrowserPopupEvidence {
  constructor(limit = 128) {
    this.limit = limit;
    this.targets = new Map();
  }
  clear() { this.targets.clear(); }
  async capture(browser, tab, actionId, operation) {
    if (typeof actionId !== "string" || !actionId) return operation(() => {});
    const connection = await browser.ensureConnection();
    let dispatched = false;
    const off = connection.subscribe(message => {
      const target = message.params?.targetInfo;
      if (!dispatched || message.method !== "Target.targetCreated" || target?.type !== "page"
        || target.openerId !== tab.target_id || typeof target.targetId !== "string") return;
      this.targets.set(target.targetId, { actionId, seen: false });
      while (this.targets.size > this.limit) this.targets.delete(this.targets.keys().next().value);
    });
    try {
      return await operation(() => {
        dispatched = true;
        return () => { dispatched = false; };
      });
    }
    catch (error) {
      for (const [target, evidence] of this.targets) {
        if (evidence.actionId === actionId) this.targets.delete(target);
      }
      throw error;
    }
    finally { off(); }
  }
  inventory(tabs) {
    const live = new Set(tabs.map(tab => tab.target_id));
    for (const [target, evidence] of this.targets) {
      if (live.has(target)) evidence.seen = true;
      else if (evidence.seen) this.targets.delete(target);
    }
    return Object.fromEntries(tabs.flatMap(tab => {
      const evidence = this.targets.get(tab.target_id);
      return evidence ? [[tab.tab_id, evidence.actionId]] : [];
    }));
  }
}
