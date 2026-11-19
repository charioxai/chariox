// Where a notification goes: the person (a host notification) or an automation the kernel made for an agent, a Room's
// agent or a workflow on the App's one outgoing event. The kernel decides whom a bind acts for; an agent binds itself.
import { OUTGOING } from '../event-types.mjs';

export function createTargets({ routing, events, host }) {
  return {
    /** The kernel automation of a target, made once; the same agent and event answer the same automation. */
    async bind(target) {
      if (target.kind === 'person') return null;
      const bound = target.kind === 'workflow'
        ? await routing.bindWorkflow(OUTGOING, target.sessionId, target.publicationRef, target.queueRef ?? null)
        : await routing.bindAgent(OUTGOING, target.agentId);
      return bound.automationId;
    },

    /** An automation goes only when no other subscription still uses it. */
    async release(automationId, subscriptions) {
      if (automationId && !subscriptions.some(subscription => subscription.automationId === automationId)) {
        await routing.unbindTarget(automationId).catch(error => { if ((error?.payload ?? error)?.code !== 'TARGET_NOT_FOUND') throw error; });
      }
    },

    async deliver(subscription, notification, key) {
      if (subscription.target.kind === 'person') {
        await host.notify(notification.title, notification.text);
        return;
      }
      const occurredAtMs = BigInt(notification.at);
      await events.emit({
        automationId: subscription.automationId, occurrenceId: events.occurrenceId(`github:${subscription.id}:${key}`, occurredAtMs),
        eventVersion: 1, occurredAtMs, scheduleRevision: null,
        payload: JSON.stringify(notification.data), invocation: { prompt: notification.text, artifacts: [] },
      });
    },
  };
}
