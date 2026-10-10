// MD-DISPLAY-02/04: local-clock stage breakdown; enclosing spans are not additive.
import { distribution } from './drill-metrics.mjs';
export function summarizeStages(receipt) {
  const samples = new Map(), add = (name, value) => {
    if (!Number.isFinite(value)) throw Error('MD-DISPLAY: non-finite timing');
    if (!samples.has(name)) samples.set(name, []);
    samples.get(name).push(value);
  };
  for (const probe of [...(receipt.probes ?? []),...(receipt.type_probes??[])]) {
    const spans = [...(receipt.host_timings ?? []), ...(receipt.kernel_timings ?? []), ...(receipt.client_timings ?? [])]
      .filter(span => span.started_ms >= probe.started_ms - 2 && span.ended_ms <= Math.max(probe.presented_ms,probe.credit_released_ms??0) + 2);
    for (const span of spans) add(span.stage, span.duration_ms);
    add('input_to_draw', probe.drawn_ms - probe.started_ms);
    add('draw_to_raf', probe.presented_ms - probe.drawn_ms);
    add('input_to_raf', probe.presented_ms - probe.started_ms);
    const queue = spans.find(span => span.stage === 'event_queue_credit');
    const arrived = spans.find(span => span.stage === 'event_received');
    if (queue && arrived) add('queued_event_to_viewer', arrived.started_ms - queue.ended_ms);
  }
  return {
    note: 'MD-DISPLAY-02/04: same-builder epoch clocks. Nested spans are not additive; queued_event_to_viewer includes writer queue and both local relay hops. rAF after canvas draw is a software presentation proxy, not physical photon timing.',
    ...(receipt.type_probes?.length?{typing:summarizeStages({...receipt,probes:receipt.type_probes,type_probes:[]}).stages}:{}),
    stages: Object.fromEntries([...samples].map(([name, values]) => [name, distribution(values)])),
  };
}
