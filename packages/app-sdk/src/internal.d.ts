import type { Duplex } from 'node:stream';
import type { CallOptions, InvocationContext, Json } from './index.js';

export const APP_WIRE_VERSION: 1;
export const MAX_FRAME_BYTES: number;
interface Envelope { version: 1; generation: string }
export type AppMessage = Envelope & (
  | { kind: 'request'; id: string; method: string; params: Json; deadline_ms: number; context?: Record<string, Json> }
  | { kind: 'response'; id: string; result: Json }
  | { kind: 'response'; id: string; error: { code: string; message: string; retryable?: boolean } }
  | { kind: 'cancel'; id: string }
  | { kind: 'event'; name: string; data: Json }
);
export interface AppTransport {
  send(message: AppMessage): void;
  subscribe(onMessage: (message: AppMessage) => void, onClose: (reason?: Error) => void): () => void;
  close(reason?: Error): void;
}
export interface TransportOptions { maxFrameBytes?: number; maxQueuedBytes?: number; frameTimeoutMs?: number }
export function inheritedTransport(fd?: number, options?: TransportOptions): AppTransport;
export function streamTransport(stream: Duplex, options?: TransportOptions): AppTransport;
export function encodeFrame(message: AppMessage, limit?: number): Buffer;
export function validateMessage(message: unknown, sender?: 'worker' | 'supervisor'): AppMessage;
export class FrameDecoder {
  constructor(onMessage: (message: AppMessage) => void, limit?: number);
  push(chunk: Uint8Array): void;
  readonly hasPartialFrame: boolean;
  end(): void;
  close(): void;
}
export class AppPeer {
  constructor(options: {
    transport: AppTransport;
    generation: string;
    handleRequest?: (method: string, params: Json, context: InvocationContext) => Json | void | Promise<Json | void>;
    onControlEvent?: (name: string, data: Json) => void;
    limits?: { maxPending?: number; maxHandlers?: number; maxDeadlineMs?: number };
  });
  request(method: string, params: Json, options?: CallOptions): Promise<Json>;
  close(reason?: Error): void;
}
