// UTF-8 only: fail explicitly for other labels rather than corrupting text.
export class TextEncoder {
  get encoding() { return 'utf-8'; }
  encode(text = '') {
    const bytes = [];
    for (const character of String(text)) {
      let point = character.codePointAt(0);
      if (point >= 0xd800 && point <= 0xdfff) point = 0xfffd;
      if (point < 0x80) bytes.push(point);
      else if (point < 0x800) bytes.push(0xc0 | (point >> 6), 0x80 | (point & 63));
      else if (point < 0x10000) bytes.push(0xe0 | (point >> 12), 0x80 | ((point >> 6) & 63), 0x80 | (point & 63));
      else bytes.push(0xf0 | (point >> 18), 0x80 | ((point >> 12) & 63), 0x80 | ((point >> 6) & 63), 0x80 | (point & 63));
    }
    return Uint8Array.from(bytes);
  }
  encodeInto(text, destination) {
    if (!(destination instanceof Uint8Array)) throw new TypeError('destination must be Uint8Array');
    let read = 0, written = 0;
    for (const character of String(text)) {
      const bytes = this.encode(character);
      if (written + bytes.length > destination.length) break;
      destination.set(bytes, written);
      written += bytes.length;
      read += character.length;
    }
    return { read, written };
  }
}

export class TextDecoder {
  #pending = new Uint8Array();
  #bom = false;
  constructor(label = 'utf-8', { fatal = false, ignoreBOM = false } = {}) {
    if (!['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(String(label).trim().toLowerCase())) {
      throw new RangeError('the App SDK supports UTF-8 only');
    }
    this.fatal = Boolean(fatal);
    this.ignoreBOM = Boolean(ignoreBOM);
  }
  get encoding() { return 'utf-8'; }
  decode(input = new Uint8Array(), { stream = false } = {}) {
    const bytes = input instanceof ArrayBuffer ? new Uint8Array(input)
      : ArrayBuffer.isView(input) ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength)
        : (() => { throw new TypeError('input must be a BufferSource'); })();
    const all = new Uint8Array(this.#pending.length + bytes.length);
    all.set(this.#pending); all.set(bytes, this.#pending.length);
    this.#pending = new Uint8Array();
    let text = '', i = 0;
    const invalid = () => {
      if (this.fatal) {
        this.#pending = new Uint8Array(); this.#bom = false;
        throw new TypeError('invalid UTF-8');
      }
      return '\ufffd';
    };
    while (i < all.length) {
      const first = all[i];
      const length = first < 0x80 ? 1 : first >= 0xc2 && first <= 0xdf ? 2
        : first >= 0xe0 && first <= 0xef ? 3 : first >= 0xf0 && first <= 0xf4 ? 4 : 0;
      if (!length) { text += invalid(); i++; continue; }
      let point = first & (length === 1 ? 127 : (1 << (7 - length)) - 1);
      let used = 1;
      for (; used < length && i + used < all.length; used++) {
        const next = all[i + used];
        if (next < 0x80 || next > 0xbf || (used === 1 && (
          (first === 0xe0 && next < 0xa0) || (first === 0xed && next > 0x9f)
          || (first === 0xf0 && next < 0x90) || (first === 0xf4 && next > 0x8f)))) break;
        point = (point << 6) | (next & 63);
      }
      if (used < length) {
        if (i + used === all.length && stream) { this.#pending = all.slice(i); break; }
        text += invalid(); i += used; continue;
      }
      text += String.fromCodePoint(point); i += length;
    }
    if (text.length && !this.#bom) {
      this.#bom = true;
      if (!this.ignoreBOM && text.charCodeAt(0) === 0xfeff) text = text.slice(1);
    }
    if (!stream) this.#bom = false;
    return text;
  }
}
