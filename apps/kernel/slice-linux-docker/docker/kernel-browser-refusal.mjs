// Host-only policy errors; page exceptions and arbitrary error codes never qualify.
const reasons = new Set(['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']);
export class UserDomainRefusal extends Error {
  constructor(reason) {
    if (!reasons.has(reason)) throw new TypeError('Unknown user-domain refusal reason');
    super('User-domain request refused');
    this.code = 'user_domain_'+reason;
  }
}
// MP-08/MP-10/MP-11: a fixed, pre-dispatch coordinate refusal an agent can act on.
export class PointerOutsideViewport extends Error {
  constructor() { super('MD-2: pointer outside viewport; x/y are CSS pixels inside state viewport css_width x css_height (screenshot pixels / device_scale_factor)'); }
}
