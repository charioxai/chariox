// A GitHub-flavoured Markdown subset for pull request text. Text from GitHub is untrusted and the page can call
// the App, so nothing here ever becomes HTML: parse() makes a tree of plain data, render() builds nodes with
// textContent (markdown-render.mjs). Raw HTML in the text is shown as text.
export function parseInline(text) {
  const out = [];
  let rest = text;
  const rules = [
    [/^`([^`]+)`/, m => ({ t: 'code', text: m[1] })],
    [/^\*\*([^*]+)\*\*|^__([^_]+)__/, m => ({ t: 'strong', children: parseInline(m[1] ?? m[2]) })],
    [/^~~([^~]+)~~/, m => ({ t: 'del', children: parseInline(m[1]) })],
    [/^\*([^*\s][^*]*)\*|^_([^_\s][^_]*)_/, m => ({ t: 'em', children: parseInline(m[1] ?? m[2]) })],
    [/^!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/, m => ({ t: 'image', text: m[1] })],
    [/^\[([^\]]+)\]\(([^)\s]+)[^)]*\)/, m => ({ t: 'link', href: m[2], children: parseInline(m[1]) })],
    [/^<(https?:\/\/[^>\s]+)>/, m => ({ t: 'link', href: m[1], children: [{ t: 'text', text: m[1] }] })],
    [/^https?:\/\/[^\s<>)\]]+/, m => ({ t: 'link', href: m[0], children: [{ t: 'text', text: m[0] }] })],
    [/^@([A-Za-z0-9][A-Za-z0-9-]{0,38})\b/, m => ({ t: 'mention', text: m[0] })],
    [/^#(\d+)\b/, m => ({ t: 'ref', text: m[0] })],
  ];
  let buffer = '';
  const flush = () => { if (buffer) { out.push({ t: 'text', text: buffer }); buffer = ''; } };
  while (rest) {
    let hit = null;
    for (const [pattern, make] of rules) {
      const match = pattern.exec(rest);
      if (match) { hit = [match, make(match)]; break; }
    }
    if (hit && !(hit[1].t === 'ref' && /\w$/.test(buffer)) && !(hit[1].t === 'mention' && /\w$/.test(buffer))) {
      flush(); out.push(hit[1]); rest = rest.slice(hit[0][0].length);
    } else { buffer += rest[0]; rest = rest.slice(1); }
  }
  flush();
  return out;
}

const isTableRow = line => /^\s*\|.*\|\s*$/.test(line);
const cells = line => line.trim().replace(/^\||\|$/g, '').split('|').map(cell => cell.trim());

export function parse(source) {
  const lines = String(source ?? '').replace(/\r\n?/g, '\n').split('\n');
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    const fence = /^\s*(```+|~~~+)\s*([\w+-]*)/.exec(line);
    if (fence) {
      const body = [];
      for (i++; i < lines.length && !lines[i].trim().startsWith(fence[1]); i++) body.push(lines[i]);
      i++;
      blocks.push({ t: 'fence', lang: fence[2], text: body.join('\n') });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*?)\s*#*$/.exec(line);
    if (heading) { blocks.push({ t: 'heading', level: heading[1].length, children: parseInline(heading[2]) }); i++; continue; }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { blocks.push({ t: 'rule' }); i++; continue; }
    if (/^\s*>/.test(line)) {
      const quoted = [];
      for (; i < lines.length && /^\s*>/.test(lines[i]); i++) quoted.push(lines[i].replace(/^\s*> ?/, ''));
      blocks.push({ t: 'quote', children: parse(quoted.join('\n')) });
      continue;
    }
    if (isTableRow(line) && i + 1 < lines.length && /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(lines[i + 1])) {
      const head = cells(line);
      const rows = [];
      for (i += 2; i < lines.length && isTableRow(lines[i]); i++) rows.push(cells(lines[i]));
      blocks.push({ t: 'table', head: head.map(parseInline), rows: rows.map(row => row.map(parseInline)) });
      continue;
    }
    const item = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(line);
    if (item) {
      const ordered = /\d/.test(item[2]);
      const items = [];
      for (; i < lines.length; i++) {
        const next = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/.exec(lines[i]);
        if (!next || /\d/.test(next[2]) !== ordered || next[1].length > item[1].length + 1) break;
        const task = /^\[([ xX])\]\s+(.*)$/.exec(next[3]);
        items.push({ checked: task ? task[1] !== ' ' : null, children: parseInline(task ? task[2] : next[3]) });
      }
      blocks.push({ t: 'list', ordered, items });
      continue;
    }
    const paragraph = [];
    for (; i < lines.length && lines[i].trim() && !/^\s*(```|~~~|#{1,6}\s|>)/.test(lines[i]); i++) paragraph.push(lines[i]);
    blocks.push({ t: 'paragraph', lines: paragraph.map(parseInline) });
  }
  return blocks;
}
