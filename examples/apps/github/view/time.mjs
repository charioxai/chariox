// Times as GitHub words them: "3 hours ago", "yesterday", "on Nov 14" and, for another year, "on Nov 14, 2025".
const MINUTE = 60e3, HOUR = 60 * MINUTE, DAY = 24 * HOUR;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const plural = (n, unit) => `${n} ${unit}${n === 1 ? '' : 's'} ago`;

export function ago(iso, now = Date.now()) {
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return '';
  const age = Math.max(now - then, 0);
  if (age < MINUTE) return 'just now';
  if (age < HOUR) return plural(Math.floor(age / MINUTE), 'minute');
  if (age < DAY) return plural(Math.floor(age / HOUR), 'hour');
  if (age < 2 * DAY) return 'yesterday';
  if (age < 30 * DAY) return plural(Math.floor(age / DAY), 'day');
  const date = new Date(then), year = date.getUTCFullYear() === new Date(now).getUTCFullYear() ? '' : `, ${date.getUTCFullYear()}`;
  return `on ${MONTHS[date.getUTCMonth()]} ${date.getUTCDate()}${year}`;
}

/** "5s", "2m 3s": how long a check ran. */
export function duration(startIso, endIso) {
  const ms = Date.parse(endIso) - Date.parse(startIso);
  if (!Number.isFinite(ms) || ms < 0) return '';
  const seconds = Math.round(ms / 1000);
  return seconds < 60 ? `${seconds}s` : seconds < 3600 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`;
}

export const clock = ms => new Date(ms).toISOString().slice(11, 16);
