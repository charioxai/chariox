// Host-only policy errors; page exceptions and arbitrary error codes never qualify.
const reasons = new Set(['not_focused_agent','foreign_owner','stale_epoch','stale_reference','not_granted','sensitive_requires_focus']);
export class UserDomainRefusal extends Error {
  constructor(reason) {
    if (!reasons.has(reason)) throw new TypeError('Unknown user-domain refusal reason');
    super('User-domain request refused');
    this.code = 'user_domain_'+reason;
  }
}
// Region capture failures the person can act on. Not policy refusals: the code and text reach the viewer as written.
const captureMessages = {
  page_unavailable: 'The page is not ready to capture; try again in a moment',
  view_resized: 'The page view changed size; select the region again',
};
export class CaptureRefusal extends Error {
  constructor(reason) {
    super(captureMessages[reason]);
    this.code = 'capture_' + reason;
  }
}
