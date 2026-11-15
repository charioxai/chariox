// An App reuses its own components; only their bounded pixels become the card's
// view. No App HTML, text or controls become kernel-written card framing.
export function createConfirmationViews() {
  const renderers = new Map();
  let root = null;
  const clear = () => { root?.remove(); root = null; };
  return Object.freeze({
    register(name, render) {
      if (!/^[a-z][a-z0-9_]{0,63}$/.test(name) || typeof render !== 'function') throw new TypeError('chariox.view.confirmation needs a name and a component renderer');
      if (renderers.has(name)) throw new TypeError('chariox.view.confirmation: this view is already registered');
      renderers.set(name, render);
    },
    async prepare(name, input) {
      clear();
      const render = renderers.get(name);
      if (!render) throw new Error('The App has not registered its confirmation view');
      const element = await render(JSON.parse(JSON.stringify(input)));
      if (element?.nodeType !== 1 || element.ownerDocument !== document) throw new TypeError('A confirmation renderer must return an element from this App view');
      if (document.querySelector('[data-chariox-confirmation]')) throw new Error('The App already contains a confirmation view');
      root = document.createElement('section');
      root.setAttribute('data-chariox-confirmation', '');
      root.inert = true;
      // Paint containment also clips positioned descendants to this App's view.
      const style = { position:'fixed', left:'16px', top:'16px', width:'min(480px, calc(100vw - 32px))', maxHeight:'320px', overflow:'auto', contain:'layout paint', boxSizing:'border-box', zIndex:'2147483647' };
      for (const [name, value] of Object.entries(style)) root.style.setProperty(name.replace(/[A-Z]/g, letter => '-' + letter.toLowerCase()), value, 'important');
      root.append(element);
      document.body.append(root);
      const box = root.getBoundingClientRect();
      if (box.width <= 0 || box.height <= 0 || box.width > 480 || box.height > 320) { clear(); throw new Error('The App confirmation must fit a 480 by 320 pixel card'); }
      return { selector:'[data-chariox-confirmation]' };
    },
    clear,
  });
}
