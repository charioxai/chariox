// MP-08/MP-10: protocol 629: the pages this browser profile visited, for the mirror's `:visited` links. Chrome hides
// `:visited` from page script, so only the kernel, which sees the profile's navigations, can tell a visited link.
// Like Chrome's partitioned visited links, a visit counts on the origin that started it (the page whose link was
// followed); a navigation no page started (a typed URL) counts on its own origin. Keys are `<origin> <exact URL>`,
// the most recent 4096, in memory for the life of the kernel browser host.
export const VISITED_LINKS = 4096;

export class VisitedLinks {
  constructor(limit = VISITED_LINKS) { this.keys = new Set(); this.limit = limit; this.requested = new Map(); this.listeners = new Set(); }
  // The URLs visited from `origin`, for a snapshot of a page on that origin.
  from(origin) { const prefix = `${origin} `, out = []; for (const key of this.keys) if (key.startsWith(prefix)) out.push(key.slice(prefix.length)); return out; }
  record(origin, url) {
    if (!/^https?:\/\//.test(origin ?? '') || !/^https?:\/\//.test(url ?? '')) return;
    const key = `${origin} ${url}`, fresh = !this.keys.delete(key);
    this.keys.add(key);
    if (this.keys.size > this.limit) this.keys.delete(this.keys.values().next().value);
    if (fresh) for (const listener of this.listeners) listener(origin, url);
  }
  // CDP Page events of a page target's main frame (its frame id is the target id); `pageUrl` is the page it shows. A
  // page asks for its navigations first (`frameRequestedNavigation`), so a commit without a request of its own was
  // started by no page. A new-tab request commits in another target: it counts now.
  observe(targetId, message, pageUrl) {
    const params = message?.params;
    if (message?.method === 'Page.frameRequestedNavigation' && params?.frameId === targetId) {
      const origin = originOf(pageUrl);
      if (params.disposition && params.disposition !== 'currentTab') this.record(origin, params.url);
      else this.requested.set(targetId, { url: params.url, origin });
    } else if (message?.method === 'Page.frameNavigated' && !params?.frame?.parentId) {
      const url = `${params.frame.url}${params.frame.urlFragment ?? ''}`, request = this.requested.get(targetId);
      this.requested.delete(targetId);
      // A redirected request is visited as asked and as committed (Chrome marks the whole chain).
      if (request) { this.record(request.origin, request.url); this.record(request.origin, url); } else this.record(originOf(url), url);
    } else if (message?.method === 'Page.navigatedWithinDocument' && params?.frameId === targetId) {
      this.record(originOf(params?.url), params?.url);
    }
  }
}

const originOf = url => { try { const origin = new URL(url).origin; return origin === 'null' ? null : origin; } catch { return null; } };
