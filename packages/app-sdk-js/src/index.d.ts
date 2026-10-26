export interface AppError { code: string; message: string; retryable: boolean | null }
export interface InvocationContext {
  deadlineMs: bigint; installationId: string | null; roomId: string | null;
  operationId: string | null; agentId: string | null; taskId: string | null; turnId: string | null;
  actor: { kind: 'human' | 'agent' | 'background'; id: string } | null;
}
export interface Receipt { receiptId: string; state: string }
export interface Wake { id: string; dueAtMs: bigint; revision: string | null }
export interface Occurrence {
  automationId: string; occurrenceId: string; eventVersion: number; occurredAtMs: bigint;
  payload: string; invocation: { prompt: string; artifacts: Array<{ name: string; mediaType: string; reference: string; sizeBytes: bigint | null; digest: string | null }> };
  scheduleRevision: string | null;
}
export interface LiveEvent { name: string; data: string }
export interface HostStream<T> extends AsyncIterableIterator<T> {
  return(): Promise<IteratorResult<T>>;
}
export class ResourceScope {
  readonly signal: AbortSignal;
  own<T>(pending: Promise<T>, dispose: (value: T) => void | Promise<void>): Promise<T>;
  release(value: unknown): Promise<void>;
  close(reason?: string): Promise<void>;
}
export interface AppContext extends InvocationContext {
  signal: AbortSignal; resources: ResourceScope;
  fetch: typeof fetch;
  subscribe(name: string): Promise<HostStream<LiveEvent>>;
}
export interface AppDefinition {
  // Input schemas belong in the signed declarations; apps choose their input
  // types here and the kernel validates those declarations at the boundary.
  tools?: Record<string, (input: any, context: AppContext) => unknown | Promise<unknown>>;
  events?: Record<string, (input: any, context: AppContext & { occurrenceId: string }) => unknown | Promise<unknown>>;
  lifecycle?: Record<string, (input: any, context: AppContext) => unknown | Promise<unknown>>;
  wake?: (wake: Wake, context: AppContext & { overdue: boolean }) => void | Promise<void>;
}
export interface FutureReader<T> { read(): Promise<T>; cancelRead(): void; drop(): void }
export interface Handlers {
  register(): void;
  callTool(name: string, input: string, context: InvocationContext, cancel: FutureReader<string>): Promise<string>;
  deliverEvent(name: string, occurrenceId: string, payload: string, context: InvocationContext, cancel: FutureReader<string>): Promise<string>;
  lifecycle(event: string, data: string | null, context: InvocationContext, cancel: FutureReader<string>): Promise<string | null>;
  deliverWake(wake: Wake, overdue: boolean, context: InvocationContext, cancel: FutureReader<string>): Promise<void>;
}
export function defineApp(app: AppDefinition): Handlers;
export const registry: { tool(name: string): void; event(name: string): void; lifecycle(event: string): void };
export interface StateRecord { value: string; version: bigint }
export interface Transaction {
  schemaVersion: number; checks: Array<{ key: string; version: bigint | null }>;
  writes: Array<{ tag: 'put'; val: [string, string] } | { tag: 'delete'; val: string }>;
  occurrences: Occurrence[]; wakes: Array<{ tag: 'set'; val: Wake } | { tag: 'cancel'; val: string }>;
}
export const state: {
  get(key: string): Promise<StateRecord | null>;
  transact(tx: Transaction): Promise<{ revision: bigint; receipts: Receipt[] }>;
};
export const schedule: { set(wake: Wake): Promise<Wake[]>; cancel(id: string): Promise<Wake[]>; list(): Promise<Wake[]> };
export const events: {
  emit(occurrence: Occurrence): Promise<Receipt>; status(id: string): Promise<Receipt>; retry(id: string): Promise<Receipt>;
  automations(): Promise<Array<{ automationId: string; event: string; eventVersion: number; state: string; lastReceipt: Receipt | null }>>;
  occurrenceId(sourceKey: string, occurredAtMs: bigint): string;
  subscribe(name: string): Promise<HostStream<LiveEvent>>;
};
export const connections: {
  list(): Promise<Array<{ generatorId: string; connectionId: string; actions: string[] }>>;
  action(request: { connectionId: string; action: string; input: string | null; context: string | null; idempotencyKey: string | null }): Promise<{ accepted: boolean; result: string; idempotencyKey: string }>;
};
export const files: {
  atomicReplace(path: string, contents: AsyncIterable<Uint8Array>): Promise<bigint>;
  snapshot(name: string, consistency: 'quiescent' | 'crash-consistent'): Promise<{ snapshotId: string; consistency: 'quiescent' | 'crash-consistent'; files: bigint; bytes: bigint }>;
  import(grantId: string, destination: string): Promise<[bigint, string]>;
  export(path: string): Promise<string>;
};
export const http: {
  fetch(request: { url: string; method: string; headers: Array<[string, string]>; connectionId: string | null; operationId: string | null }, body: AsyncIterable<Uint8Array> | null): Promise<[{ status: number; headers: Array<[string, string]>; url: string }, HostStream<Uint8Array>]>;
};
export const log: { write(level: 'debug' | 'info' | 'warn' | 'error', message: string, fields: string): void };
export const host: {
  notify(title: string, body: string | null): Promise<string>;
  openLink(url: string): Promise<{ offerId: string; state: string }>;
  writeClipboard(text: string): Promise<{ offerId: string; state: string }>;
  pickFile(request: { multiple: boolean; accept: string[] }): Promise<string[]>;
};
export interface Operation { operationId: string; state: 'pending' | 'approved' | 'denied' | 'expired' | 'cancelled' | 'reconciliation' }
export const validation: {
  request(request: { action: string; parameters: string; operationId: string | null; connectionId: string | null }): Promise<Operation>;
  status(operationId: string): Promise<Operation>;
};
export class Headers implements Iterable<[string, string]> {
  constructor(init?: Headers | Iterable<[string, string]> | Record<string, string>);
  append(name: string, value: string): void; set(name: string, value: string): void; delete(name: string): void;
  get(name: string): string | null; has(name: string): boolean;
  entries(): IterableIterator<[string, string]>; [Symbol.iterator](): IterableIterator<[string, string]>;
}
export class AppResponse {
  readonly status: number; readonly url: string; readonly ok: boolean; readonly headers: Headers;
  readonly bodyUsed: boolean;
  readonly body: AsyncIterable<Uint8Array> & { cancel(): Promise<void> };
  bytes(options?: { limit: number }): Promise<Uint8Array>; arrayBuffer(options?: { limit: number }): Promise<ArrayBuffer>;
  text(options?: { limit: number }): Promise<string>; json(options?: { limit: number }): Promise<unknown>;
}
export interface FetchOptions {
  method?: string; headers?: Headers | Iterable<[string, string]> | Record<string, string>;
  body?: string | Uint8Array | AsyncIterable<Uint8Array>; signal?: AbortSignal;
  connectionId?: string; operationId?: string;
}
export function fetch(url: string, init?: FetchOptions): Promise<AppResponse>;
