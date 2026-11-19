// Doing something on GitHub from the window: the button says it is working, the refusal is shown in GitHub's own
// flash above the control, and a change that went through reloads what the page shows.
import { call, forget } from './api.mjs';
import { clear, h, icon } from './dom.mjs';
import { flash } from './parts.mjs';

/** What a refusal says: the App's own words (it states what to do next), never a code. */
export const reason = error => String(error?.message ?? error);

/** A place above a control where a refusal shows; `show(error)` fills it, `show()` empties it. */
export function notice() {
  const element = h('div');
  return { element, show: error => { clear(element); if (error) element.append(flash('error', icon('alert', { size: 16, className: 'mr-2' }), reason(error))); } };
}

/**
 * Runs the tool `name` and answers its result, or `undefined` when it was refused (the reason is shown in `where`).
 * The buttons given are disabled meanwhile. `after` runs once the change went through, e.g. reloading the page.
 */
export async function act(name, input, { buttons = [], where, after } = {}) {
  for (const button of buttons) button.disabled = true;
  where?.show();
  try {
    const result = await call(name, input);
    forget();
    await after?.(result);
    return result;
  } catch (error) {
    where?.show(error);
    if (!where) throw error;
    return undefined;
  } finally {
    for (const button of buttons) button.disabled = false;
  }
}
