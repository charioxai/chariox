import type { AppError } from './index.js';
export interface RouteSpec { routeId: string; event: string; repository: string; sourceEventType: string; sourceEventVersion: number }
export interface Route extends RouteSpec { active: boolean; pending: bigint; delivered: bigint; failed: bigint; expired: bigint }
export interface ConnectionState { generatorId: string; connectionId: string | null; status: string; maxRoutes: number; routes: number }
export interface Target { automationId: string; event: string; kind: 'agent' | 'workflow'; agentId: string | null; state: string }
type ImportPromise<T> = Promise<T>;
export const eventRouting: {
  ensureRoute(spec: RouteSpec): ImportPromise<Route>;
  removeRoute(routeId: string): ImportPromise<void>;
  setRouteActive(routeId: string, active: boolean): ImportPromise<Route>;
  listRoutes(): ImportPromise<Route[]>;
  connectionStatus(): ImportPromise<ConnectionState>;
  connectionResources(query: string | null, cursor: string | null, limit: number): ImportPromise<string>;
  startAuthorization(returnUrl: string | null): ImportPromise<{ authorizationId: string; url: string }>;
  bindAgent(event: string, agentId: string | null): ImportPromise<Target>;
  bindWorkflow(event: string, sessionId: string, publicationRef: string, queueRef: string | null): ImportPromise<Target>;
  unbindTarget(automationId: string): ImportPromise<void>;
  listTargets(): ImportPromise<Target[]>;
};
export type { AppError };
