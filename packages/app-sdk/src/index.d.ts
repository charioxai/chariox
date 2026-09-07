export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface CallOptions { signal?: AbortSignal; timeoutMs?: number }
export interface InvocationContext {
  readonly signal: AbortSignal;
  readonly deadlineMs: number;
  /** Context is issued by the kernel; it cannot establish a human approval. */
  readonly installation_id?: string;
  readonly room_id?: string;
  readonly operation_id?: string;
  readonly agent_id?: string;
  readonly task_id?: string;
  readonly turn_id?: string;
  readonly actor?: Readonly<{ kind: 'human' | 'agent' | 'background'; id: string }>;
}
export type Handler<Input = Json, Output = Json | void> =
  (input: Input, context: InvocationContext) => Output | Promise<Output>;
export type LifecycleEvent = 'startup' | 'suspend' | 'resume' | 'shutdown' | 'prepare_update' | 'configuration_change';

export class AppError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  constructor(code: string, message: string, options?: { retryable?: boolean; cause?: unknown });
}

export interface EventOccurrence {
  automationId: string;
  occurrenceId: string;
  eventVersion: number;
  payload: Json;
}
export interface EventReceipt {
  receiptId: string;
  state: 'accepted' | 'queued' | 'delivered' | 'retryable' | 'failed' | 'expired';
}
export interface StateRecord { value: Json; version: number }
export interface StateTransaction {
  schemaVersion: number;
  checks: { key: string; version: number | null }[];
  writes: ({ key: string; value: Json } | { key: string; delete: true })[];
  /** Kernel persistence commits these occurrences with the structured writes. */
  occurrences?: EventOccurrence[];
}
export interface HttpRequest {
  url: string;
  method?: string;
  headers?: [string, string][];
  body?: string | Uint8Array;
  connectionId?: string;
  operationId?: string;
}
export interface HttpResponse {
  status: number;
  headers: [string, string][];
  bodyBase64: string;
  url: string;
}
export interface ValidationOperation {
  operationId: string;
  state: 'pending' | 'approved' | 'denied' | 'expired' | 'cancelled' | 'reconciliation';
}
export interface OutputRequest {
  informationSet: string;
  version: number;
  taskRef: string;
  mode: 'intermediate' | 'final';
}

export interface AppSdk {
  readonly paths: Readonly<{ package: string; data: string; temporary: string }>;
  readonly tools: { register<Input = Json, Output = Json | void>(name: string, handler: Handler<Input, Output>): void };
  readonly events: {
    register<Payload = Json>(name: string, handler: Handler<Readonly<{ occurrenceId: string; payload: Payload }>>): void;
    emit(occurrence: EventOccurrence, options?: CallOptions): Promise<EventReceipt>;
    status(receiptId: string, options?: CallOptions): Promise<EventReceipt>;
    retry(receiptId: string, options?: CallOptions): Promise<EventReceipt>;
  };
  readonly lifecycle: { on(event: LifecycleEvent, handler: Handler): void };
  readonly state: {
    get(key: string, options?: CallOptions): Promise<StateRecord | null>;
    transaction(transaction: StateTransaction, options?: CallOptions): Promise<{ revision: number; receipts: EventReceipt[] }>;
  };
  readonly files: {
    atomicReplace(path: string, contents: string | Uint8Array, options?: CallOptions): Promise<{ bytesWritten: number }>;
    snapshot(request: { name: string; consistency: 'quiescent' | 'crash_consistent' }, options?: CallOptions): Promise<{ snapshotId: string }>;
    import(grantId: string, destination: string, options?: CallOptions): Promise<{ bytesWritten: number }>;
    export(path: string, options?: CallOptions): Promise<{ operationId: string }>;
  };
  readonly http: { request(request: HttpRequest, options?: CallOptions): Promise<HttpResponse> };
  readonly log: {
    write(level: 'debug' | 'info' | 'warn' | 'error', message: string, fields?: Record<string, Json>, options?: CallOptions): Promise<null>;
  };
  readonly host: {
    notify(request: { title: string; body?: string }, options?: CallOptions): Promise<{ notificationId: string }>;
    openLink(url: string, options?: CallOptions): Promise<null>;
    writeClipboard(text: string, options?: CallOptions): Promise<null>;
    pickFile(request: { multiple?: boolean; accept?: string[] }, options?: CallOptions): Promise<{ grantIds: string[] }>;
  };
  readonly validation: {
    request(request: { action: string; parameters: Json; operationId?: string; connectionId?: string }, options?: CallOptions): Promise<ValidationOperation>;
    status(operationId: string, options?: CallOptions): Promise<ValidationOperation>;
  };
  readonly outputs: {
    request(request: OutputRequest, options?: CallOptions): Promise<{ requestId: string; state: 'pending_consent' | 'pending_output' }>;
    cancel(requestId: string, options?: CallOptions): Promise<null>;
  };
  /** Completes bootstrap registration. This never asserts sandbox lockdown. */
  ready(options?: CallOptions): Promise<null>;
  close(): void;
}

/** Worker bootstrap options, supplied after the trusted launcher established containment. */
export interface AppSdkOptions {
  transport: import('./internal.js').AppTransport;
  generation: string;
  paths: { package: string; data: string; temporary: string };
  declarations?: { tools?: string[]; events?: string[] };
  limits?: { maxPending?: number; maxHandlers?: number; maxDeadlineMs?: number };
}
export function createAppSdk(options: AppSdkOptions): AppSdk;
