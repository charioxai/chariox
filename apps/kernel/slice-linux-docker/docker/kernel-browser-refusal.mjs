// Host-only policy errors; page exceptions and arbitrary error codes never qualify.
const reasons = new Set(['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']);
export class UserDomainRefusal extends Error {
  constructor(reason) {
    if (!reasons.has(reason)) throw new TypeError('Unknown user-domain refusal reason');
    super('User-domain request refused');
    this.code = 'user_domain_'+reason;
  }
}
