// Caddy's lexer treats # as a comment only at a token boundary; quotes open
// only at that boundary, and backticks do not process escapes. Preserve token
// contents and physical positions rather than interpreting configuration.
// Contract: caddyserver/caddy@4ad0c6b1970d43c367b2bace3d216608e25944df,
// caddyconfig/caddyfile/lexer.go. This is a comment mask, not a Caddy validator.
const TOKEN_SPACE = /[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/u;

export function stripCaddyComments(text) {
  const output = text.split("");
  let inToken = false;
  let quote = null;
  let escaped = false;
  for (let index = 0; index < text.length; index += 1) {
    const current = text[index];
    if (index === 0 && current === "\ufeff") continue;
    if (!escaped && quote !== "`" && current === "\\") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (quote === '"' && escaped) escaped = false;
      else if (current === quote) {
        quote = null;
        inToken = false;
        escaped = false;
      }
      continue;
    }
    // Caddy ignores CR outside quotes, including within an unquoted token.
    if (current === "\r") continue;
    if (TOKEN_SPACE.test(current)) {
      if (inToken || current === "\n") escaped = false;
      inToken = false;
      continue;
    }
    if (current === "#" && !inToken) {
      while (index < text.length && text[index] !== "\n") {
        if (text[index] !== "\r") output[index] = " ";
        index += 1;
      }
      index -= 1;
      escaped = false;
      continue;
    }
    if (!inToken && (current === '"' || current === "`")) {
      quote = current;
      continue;
    }
    if (!inToken && !escaped && current === "<") {
      // Valid heredocs are literal tokens. If a recognized opener has no
      // certain closing line, retain the remainder conservatively; masking
      // its # lines could hide active content in an incomplete/uncertain view.
      const opening = /^<\r*<([^\n]*)\n/.exec(text.slice(index));
      const marker = opening?.[1].replaceAll("\r", "");
      if (marker && /^[A-Za-z0-9_-]+$/.test(marker)) {
        const bodyStart = index + opening[0].length;
        // Upstream ends the token at the first complete marker, before
        // reading the next character. A quote/token may start immediately;
        // requiring a marker-only line can skip into that token's contents.
        const closing = text.indexOf(marker, bodyStart);
        if (closing < 0) break;
        index = closing + marker.length - 1;
        escaped = false;
        continue;
      }
    }
    inToken = true;
    escaped = false;
  }
  return output.join("");
}
