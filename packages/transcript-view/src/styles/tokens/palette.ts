export const sharedPaletteStyles = `
:root {
  color-scheme: dark light;
  /* Grayscale foundation. Semantic status tokens below intentionally retain hue. */
  --accent-neutral: #d6d6d6;
  --accent-neutral-strong: #f5f5f5;
  --accent-neutral-ink: #101010;
  --brand-black: #080808;
  --terminal-startup-text: #f2f2f2;
  --terminal-startup-surface: #181818;
  --terminal-startup-line: #292929;
  --brand-ink: #111111;
  --brand-white: #f5f5f5;
  --brand-paper: #fafafa;
  --font-display: "ABC Diatype Mono", "IBM Plex Mono", "SF Mono", "Berkeley Mono", ui-monospace, monospace;
  --font-body: "ABC Diatype", "Suisse Intl", "Helvetica Neue", ui-sans-serif, system-ui, sans-serif;
  --room-workflows-min-inline-size: 18rem;
  --room-workflows-composer-size: 5rem;
  --room-workflows-ack-size: 6rem;
  --radius: 8px;
  --radius-xs: 3px;
  --radius-sm: 5px;
  --radius-md: 8px;
  --radius-full: 9999px;
  --motion-fast: 160ms;
  --motion-med: 260ms;
  --motion-slow: 900ms;
  --motion-working-sweep: 3.2s;

  /* Type scale */
  --text-2xs: 10px;
  --text-xs: 11px;
  --text-sm: 12px;
  --text-md: 13px;
  --text-lg: 14px;
  --text-xl: 18px;
  --text-2xl: 22px;
  --text-3xl: 28px;

  /* Spacing scale */
  --space-1: 4px;
  --space-2: 8px;
  --space-3: 12px;
  --space-4: 16px;
  --space-5: 24px;
  --space-6: 32px;

  /* Shared control geometry */
  --prompt-config-label-min-width: 5rem;
  --control-height-sm: 24px;
  --control-height: 28px;
  --control-height-lg: 36px;
  --control-padding-inline: var(--space-3);
  --control-font-size: var(--text-xs);

  /* Z-index scale */
  --z-raised: 1;
  --z-overlay: 5;
  --z-sidebar: 10;
  --z-dropdown: 20;
  --z-modal: 50;
  --z-toast: 100;
  /* Bottom offset of the fixed Note/Notes/Capture widgets; lifted above the prompt composer at runtime. */
  --floating-dock-bottom: 1rem;
  --toast-clearance: 0px;

  /* Semantic color tokens */
  --color-success: #7fd88f;
  --color-success-bg: color-mix(in srgb, #7fd88f 7%, transparent);
  --color-success-border: color-mix(in srgb, #7fd88f 52%, var(--line));
  --color-success-dim: color-mix(in srgb, #7fd88f 44%, var(--line));
  --color-done: #9dccff;
  --color-done-bg: color-mix(in srgb, #9dccff 8%, transparent);
  --color-done-border: color-mix(in srgb, #9dccff 58%, var(--line));
  --color-warning: #ffb86b;
  --color-warning-bg: color-mix(in srgb, #ffb86b 8%, transparent);
  --color-warning-border: color-mix(in srgb, #ffb86b 64%, var(--line));
  --color-danger: #ff8a8a;
  --color-danger-bg: color-mix(in srgb, #ff6b6b 7%, transparent);
  --color-danger-border: color-mix(in srgb, #ff6b6b 58%, var(--line));
  --color-added: #8fd8a8;
  --color-added-bg: color-mix(in srgb, #2e7d45 28%, transparent);
  --color-removed: #ff8a8a;
  --color-removed-bg: color-mix(in srgb, #7d2e2e 24%, transparent);
  --color-collaborator: #9fbdff;
  --color-collaborator-border: color-mix(in srgb, #7aa7ff 54%, var(--line));
  --color-collaborator-bg: color-mix(in srgb, #7aa7ff 7%, transparent);

  /* Syntax highlighting tokens */
  --syntax-keyword: #dedede;
  --syntax-string: #c9c9c9;
  --syntax-number: #a8a8a8;

  /* Send / action button */
  --send-bg: #dedede;
  --send-bg-hover: #f5f5f5;
  --send-text: #101010;
  --send-stop-bg: var(--color-danger);
  --send-stop-bg-hover: color-mix(in srgb, var(--color-danger) 82%, var(--text));

  /* Context meter gradient */
  --meter-low: #757575;
  --meter-high: #dedede;

  /* Fix for previously undefined variable references */
  --font-mono: var(--font-display);
  --surface-2: var(--surface-raised);
  --surface-sunken: var(--page-soft);
  --danger: var(--color-danger);
  --focus-ring: var(--accent-neutral);
  --color-info: #8fd6ff;
  --accent: var(--accent-neutral);

  /* Glow effects */
  --glow-accent: 0 0 16px color-mix(in srgb, var(--accent-neutral) 20%, transparent);
  --glow-accent-strong: 0 0 26px color-mix(in srgb, var(--accent-neutral) 38%, transparent);
}

:root,
:root[data-theme="dark"] {
  --page: #080808;
  --page-soft: #101010;
  --text: #f2f2f2;
  --muted: #a6a6a6;
  --subtle: #707070;
  --line: #292929;
  --line-strong: #3b3b3b;
  --surface: #121212;
  --surface-raised: #181818;
  --surface-card: #202020;
  --button-bg: #f2f2f2;
  --button-text: #080808;
  --footer-surface: rgb(11 11 11 / 92%);
  --footer-surface-opaque: #0b0b0b;
  --footer-surface-strong: rgb(15 15 15 / 94%);
  --shadow-xs: 0 1px 2px rgb(0 0 0 / 20%);
  --shadow: 0 24px 80px rgb(0 0 0 / 38%);
  --shadow-lg: 0 18px 70px color-mix(in srgb, black 42%, transparent);
  --shadow-xl: 0 22px 90px color-mix(in srgb, black 52%, transparent);
  --grid-main: 16%;
  --grid-cross: 12%;
}

@media (prefers-color-scheme: light) {
  :root:not([data-theme]) {
    --page: #f7f7f7;
    --page-soft: #ebebeb;
    --text: #111111;
    --muted: #5f5f5f;
    --subtle: #858585;
    --line: #dedede;
    --line-strong: #c9c9c9;
    --surface: #ffffff;
    --surface-raised: #f0f0f0;
    --button-bg: #111111;
    --button-text: #f7f7f7;
    --footer-surface: rgb(247 247 247 / 96%);
    --footer-surface-opaque: #f7f7f7;
    --footer-surface-strong: rgb(240 240 240 / 98%);
    --shadow: 0 24px 80px rgb(17 17 17 / 8%);
    --grid-main: 62%;
    --grid-cross: 44%;
  }
}

:root[data-theme="light"] {
  color-scheme: light;
  --page: #f7f7f7;
  --page-soft: #ebebeb;
  --text: #111111;
  --muted: #5f5f5f;
  --subtle: #858585;
  --line: #dedede;
  --line-strong: #c9c9c9;
  --surface: #ffffff;
  --surface-raised: #f0f0f0;
  --button-bg: #111111;
  --button-text: #f7f7f7;
  --footer-surface: rgb(247 247 247 / 96%);
  --footer-surface-opaque: #f7f7f7;
  --footer-surface-strong: rgb(240 240 240 / 98%);
  --shadow: 0 24px 80px rgb(17 17 17 / 8%);
  --grid-main: 62%;
  --grid-cross: 44%;
}

`
