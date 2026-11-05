// The node tests run the view's code without a page: give it web storage.
const items = new Map();
globalThis.localStorage = {
  getItem: key => items.get(key) ?? null,
  setItem: (key, value) => { items.set(key, String(value)); },
  removeItem: key => { items.delete(key); },
};
