import { createEventsApp } from './events.mjs';
import { createAccounts } from './accounts.mjs';
import { createGitHub } from './github.mjs';
import { createPulls } from './pulls.mjs';
import { createStatus } from './status.mjs';
import { createView } from './view.mjs';
import { tools as catalog, uiActionsDocument } from './catalog.mjs';

export function createApp({ state, events, now }) {
  const view = createView({
    state, actions: uiActionsDocument().uiActions,
    snapshot: async () => ({ accounts: await accounts.settings() }),
  });
  const accounts = createAccounts({ state, view });
  const eventsApp = createEventsApp({
    state, events,
    changed: delta => view.transactWithChange({ schemaVersion: 0, checks: [], writes: [], occurrences: [], wakes: [] }, delta),
  });
  const services = ctx => {
    const github = createGitHub({ state, fetch: ctx.fetch, now });
    return { github, ...createStatus({ accounts, github }), ...createPulls({ accounts, github }) };
  };

  // An account is added only once GitHub accepts it: the owner has bound the label and its token works.
  const accountTools = {
    async add_account({ account }, ctx) {
      await services(ctx).github.get(account, '/user', {}, { fresh: true });
      await accounts.add(account);
      return services(ctx).list_accounts();
    },
    async remove_account({ account }, ctx) { await accounts.remove(account); return services(ctx).list_accounts(); },
    async set_default_account({ account }, ctx) { await accounts.setDefault(account); return services(ctx).list_accounts(); },
    async set_repo_account({ repo, account }, ctx) { await accounts.setRepo(repo, account); return services(ctx).list_accounts(); },
  };
  const reads = Object.fromEntries(catalog.filter(tool => tool.effect === 'read').map(({ name }) => [name, (input, ctx) => services(ctx)[name](input ?? {}, ctx)]));

  return {
    tools: { ...eventsApp.tools, ...reads, ...accountTools, ...view.tools },
    events: eventsApp.events,
  };
}
