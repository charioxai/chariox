// PostgreSQL source preserves quoted values, including dollar quotes. Comments
// are masked without moving physical anchors. Plain-string backslash behavior
// depends on standard_conforming_strings, so retain source visible under either
// setting; uncertain comment candidates stay available for semantic inspection.
export function stripPostgresComments(text) {
  const standard = commentMask(text, false);
  const legacy = commentMask(text, true);
  return Array.from({ length: text.length }, (_, index) =>
    standard[index] && legacy[index] && text[index] !== "\n" && text[index] !== "\r"
      ? " " : text[index]).join("");
}

function commentMask(text, plainBackslashEscapes) {
  const mask = new Uint8Array(text.length);
  const dollarQuote = /\$(?:[A-Za-z_\u0080-\uFFFF][A-Za-z_0-9\u0080-\uFFFF]*)?\$/y;
  let blockDepth = 0;
  let quote = null;
  let backslashEscapes = false;
  let dollarDelimiter = null;
  for (let index = 0; index < text.length; index += 1) {
    const current = text[index];
    const next = text[index + 1];
    if (blockDepth) {
      mask[index] = 1;
      if ((current === "/" && next === "*") || (current === "*" && next === "/")) {
        blockDepth += current === "/" ? 1 : -1;
        mask[index + 1] = 1;
        index += 1;
      }
      continue;
    }
    if (dollarDelimiter) {
      if (text.startsWith(dollarDelimiter, index)) {
        index += dollarDelimiter.length - 1;
        dollarDelimiter = null;
      }
      continue;
    }
    if (quote) {
      if (backslashEscapes && current === "\\") {
        index += 1;
      } else if (current === quote) {
        if (next === quote) index += 1;
        else quote = null;
      }
      continue;
    }
    // Dollar quotes cannot begin inside an unquoted identifier (foo$tag$).
    if (current === "$" && !/[A-Za-z_0-9$\u0080-\uFFFF]/.test(text[index - 1] ?? "")) {
      dollarQuote.lastIndex = index;
      const delimiter = dollarQuote.exec(text)?.[0];
      if (delimiter) {
        dollarDelimiter = delimiter;
        index += delimiter.length - 1;
        continue;
      }
    }
    if (current === "'" || current === "\"") {
      quote = current;
      const explicitEscape = /[eE]/.test(text[index - 1] ?? "")
        && !/[A-Za-z_0-9$\u0080-\uFFFF]/.test(text[index - 2] ?? "");
      backslashEscapes = current === "'" && (explicitEscape || plainBackslashEscapes);
      continue;
    }
    if (current === "/" && next === "*") {
      blockDepth = 1;
      mask[index] = mask[index + 1] = 1;
      index += 1;
      continue;
    }
    if (current === "-" && next === "-") {
      while (index < text.length && text[index] !== "\n" && text[index] !== "\r") {
        mask[index] = 1;
        index += 1;
      }
      index -= 1;
    }
  }
  return mask;
}
