function copyTarget(record) {
  return record ? { ...record } : null;
}

export class BrowserCdpAuthorityRegistry {
  constructor() {
    this.browserGeneration = 0;
    this.nextAuthorityId = 0;
    this.current = null;
    this.targets = new Map();
    this.retiredEndpointKeys = new Map();
  }

  begin(connection) {
    if (!connection) {
      throw new TypeError("connection is required");
    }
    this.current && (this.current.active = false);
    this.targets = new Map();
    this.retiredEndpointKeys = new Map();
    const authority = {
      id: ++this.nextAuthorityId,
      connection,
      browserGeneration: ++this.browserGeneration,
      active: true,
      ready: false,
    };
    this.current = authority;
    return authority;
  }

  commit(authority) {
    if (!this.isCurrent(authority, { ready: false })) return false;
    authority.ready = true;
    return true;
  }

  abort(authority) {
    if (!authority) return;
    authority.active = false;
    if (this.current === authority) {
      this.current = null;
      this.targets = new Map();
      this.retiredEndpointKeys = new Map();
    }
  }

  retireCurrent() {
    const authority = this.current;
    this.abort(authority);
    return authority;
  }

  isCurrent(authority, { ready = true } = {}) {
    return Boolean(
      authority &&
      authority.active &&
      this.current === authority &&
      (!ready || authority.ready),
    );
  }

  currentFor(connection, options) {
    const authority = this.current;
    return authority?.connection === connection && this.isCurrent(authority, options)
      ? authority
      : null;
  }

  observeTarget(authority, targetId, endpointKey = null) {
    if (!this.isCurrent(authority, { ready: false }) || typeof targetId !== "string" || !targetId) {
      return null;
    }
    const previous = this.targets.get(targetId);
    const endpoint = typeof endpointKey === "string" && endpointKey.length > 0
      ? endpointKey
      : null;
    const retired = this.retiredEndpointKeys.get(targetId);
    if (previous && endpoint && retired?.has(endpoint)) {
      return { ...copyTarget(previous), rotated: false, stale: true };
    }
    const rotated = Boolean(previous?.endpointKey && endpoint && previous.endpointKey !== endpoint);
    if (rotated) {
      const retiredForTarget = retired ?? new Set();
      retiredForTarget.add(previous.endpointKey);
      this.retiredEndpointKeys.set(targetId, retiredForTarget);
    }
    const record = rotated
      ? {
          targetId,
          endpointKey: endpoint,
          targetGeneration: previous.targetGeneration + 1,
          documentGeneration: previous.documentGeneration + 1,
          documentId: null,
          sessionId: null,
        }
      : {
          targetId,
          endpointKey: endpoint ?? previous?.endpointKey ?? null,
          targetGeneration: previous?.targetGeneration ?? 1,
          documentGeneration: previous?.documentGeneration ?? 0,
          documentId: previous?.documentId ?? null,
          sessionId: previous?.sessionId ?? null,
        };
    this.targets.set(targetId, record);
    return { ...copyTarget(record), rotated, stale: false };
  }

  setSession(authority, targetId, sessionId) {
    const record = this.targets.get(targetId);
    if (
      !this.isCurrent(authority, { ready: true }) ||
      !record ||
      typeof sessionId !== "string" ||
      !sessionId
    ) {
      return null;
    }
    record.sessionId = sessionId;
    return copyTarget(record);
  }

  clearSession(authority, targetId, sessionId) {
    const record = this.targets.get(targetId);
    if (!this.isCurrent(authority, { ready: false }) || !record) return false;
    if (sessionId === undefined || record.sessionId === sessionId) {
      record.sessionId = null;
      return true;
    }
    return false;
  }

  removeTarget(authority, targetId) {
    if (!this.isCurrent(authority, { ready: false })) return false;
    this.targets.delete(targetId);
    this.retiredEndpointKeys.delete(targetId);
    return true;
  }

  setDocument(authority, targetId, documentId) {
    const record = this.targets.get(targetId);
    if (
      !this.isCurrent(authority, { ready: true }) ||
      !record ||
      typeof documentId !== "string" ||
      !documentId
    ) {
      return null;
    }
    if (record.documentId !== documentId) {
      record.documentGeneration += 1;
      record.documentId = documentId;
    }
    return copyTarget(record);
  }

  getTarget(authority, targetId) {
    if (!this.isCurrent(authority, { ready: true })) return null;
    return copyTarget(this.targets.get(targetId));
  }

  isTargetCurrent(authority, targetId, expected = {}) {
    const record = this.getTarget(authority, targetId);
    if (!record) return false;
    return (
      (expected.targetGeneration === undefined || record.targetGeneration === expected.targetGeneration) &&
      (expected.documentGeneration === undefined || record.documentGeneration === expected.documentGeneration) &&
      (expected.documentId === undefined || record.documentId === expected.documentId) &&
      (expected.sessionId === undefined || record.sessionId === expected.sessionId)
    );
  }

  isSessionCurrent(authority, targetId, sessionId) {
    return this.isTargetCurrent(authority, targetId, { sessionId });
  }

  snapshot() {
    return {
      browser_generation: this.browserGeneration,
      current_authority_id: this.current?.id ?? null,
      ready: this.current?.ready === true,
      targets: [...this.targets.values()].map(copyTarget),
    };
  }
}
