// Routes are a projection of the subscriptions: one kernel inbox route per distinct (repository, generator event
// type) that an enabled subscription needs. A subscription never owns a route; this reconciles the whole set.
import { incoming, kinds } from '../event-types.mjs';

const incomingName = Object.fromEntries(Object.entries(incoming).map(([name, type]) => [type, name]));
const fail = (code, message) => { throw { code, message, retryable: false }; };
const codeOf = error => (error?.payload ?? error)?.code;
export const routeId = (repo, sourceType) => `${sourceType}@${repo}`;

export const sourceTypes = subscription => [...new Set(subscription.events.flatMap(kind => kinds[kind]))];

/** Every repository of the granted connection an organization or "all" scope covers. */
async function connectionRepositories(routing) {
  const names = [];
  let cursor = null;
  do {
    const page = JSON.parse(await routing.connectionResources(null, cursor, 100)).page;
    for (const resource of page.resources) names.push(resource.connectionScope ?? resource.name);
    cursor = page.nextCursor ?? null;
  } while (cursor && names.length < 2000);
  return names;
}

export const repositoriesOf = (scope, connection) => scope.kind === 'repos' ? scope.repos
  : connection.filter(repo => scope.kind === 'all' || repo.toLowerCase().startsWith(`${scope.org.toLowerCase()}/`));

/**
 * Make the kernel's routes equal what the enabled subscriptions need.
 * Returns the live repositories and the ones the connection cannot deliver for (with why).
 */
export async function reconcile({ routing, subscriptions }) {
  const enabled = subscriptions.filter(subscription => subscription.enabled);
  const connection = enabled.some(({ scope }) => scope.kind !== 'repos') ? await connectionRepositories(routing).catch(() => []) : [];
  const wanted = new Map();
  for (const subscription of enabled) {
    for (const repo of repositoriesOf(subscription.scope, connection)) {
      for (const type of sourceTypes(subscription)) wanted.set(routeId(repo, type), { repo, type });
    }
  }
  const { maxRoutes } = await routing.connectionStatus();
  if (wanted.size > maxRoutes) fail('ROUTE_LIMIT', `These subscriptions need ${wanted.size} routes and an installation has at most ${maxRoutes}. Narrow the repositories or the events.`);
  for (const id of wanted.keys()) if (id.length > 128) fail('INVALID_ARGUMENT', `The repository name is too long to route: ${id.split('@')[1]}`);

  const existing = new Map((await routing.listRoutes()).map(route => [route.routeId, route]));
  const unrouted = new Map();
  for (const [id, { repo, type }] of wanted) {
    const route = existing.get(id);
    try {
      if (!route) await routing.ensureRoute({ routeId: id, event: incomingName[type], repository: repo, sourceEventType: type, sourceEventVersion: 1 });
      else if (!route.active) await routing.setRouteActive(id, true);
    } catch (error) {
      const code = codeOf(error);
      if (!['REPOSITORY_NOT_IN_CONNECTION', 'CONNECTION_NOT_GRANTED', 'CONNECTION_UNAVAILABLE', 'CONNECTION_AMBIGUOUS'].includes(code)) throw error;
      unrouted.set(repo, code);
    }
  }
  for (const id of existing.keys()) if (!wanted.has(id)) await routing.removeRoute(id);
  const live = [...new Set([...wanted.values()].map(({ repo }) => repo))].filter(repo => !unrouted.has(repo));
  return { live, unrouted: Object.fromEntries(unrouted) };
}
