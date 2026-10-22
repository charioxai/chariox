// Host-only policy errors; page exceptions and arbitrary error codes never qualify.
const reasons = new Set(['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']);
export class UserDomainRefusal extends Error {
  constructor(reason, race) {
    if (!reasons.has(reason)) throw new TypeError('Unknown user-domain refusal reason');
    super('User-domain request refused');
    this.code = 'user_domain_'+reason;
    // Diagnostics only: a refusal the viewer answers by rebinding (a retired region, a display it just suspended).
    if (race !== undefined) this.race = race;
  }
}
// Region capture failures the person can act on. Not policy refusals: the code and text reach the viewer as written.
const captureMessages = {
  view_resized: 'The page view changed size; select the region again',
};
export class CaptureRefusal extends Error {
  constructor(reason) {
    super(captureMessages[reason]);
    this.code = 'capture_' + reason;
  }
}
// MP-08/MP-10/MP-11: a fixed, pre-dispatch coordinate refusal an agent can act on.
export class PointerOutsideViewport extends Error {
  constructor() { super('MD-2: pointer outside viewport; x/y are CSS pixels inside state viewport css_width x css_height (screenshot pixels / device_scale_factor)'); }
}
