# CLI Themes

Chariox loads built-in themes and custom JSON themes into the same registry. The waiting room `Theme` row cycles every loaded theme with the left and right arrow keys, then persists the selected theme in `ui.theme`.

## Theme Locations

Custom themes are loaded from these directories:

- `~/.chariox/themes/*.json`
- `<workspace>/.chariox/themes/*.json`

If `XDG_CONFIG_HOME` is set, the global directory is `$XDG_CONFIG_HOME/chariox/themes/*.json`.

Project themes are loaded after global themes. A custom theme cannot replace a built-in theme ID.

## Native Format

Native themes use Chariox token names. Missing colors fall back to the OpenCode built-in theme.

```json
{
  "id": "my-sober",
  "name": "My Sober",
  "palette": {
    "primary": "#e6e6e6",
    "secondary": "#b8b8b8",
    "accent": "#d0d0d0",
    "error": "#d7d7d7",
    "warning": "#c4c4c4",
    "success": "#dadada",
    "info": "#bdbdbd",
    "text": "#e8e8e8",
    "textMuted": "#8a8a8a",
    "background": "#080808",
    "backgroundPanel": "#121212",
    "backgroundElement": "#1b1b1b",
    "border": "#3a3a3a",
    "borderActive": "#626262",
    "borderSubtle": "#2c2c2c"
  },
  "syntax": {
    "comment": "#777777",
    "keyword": "#e6e6e6",
    "function": "#d6d6d6",
    "variable": "#e8e8e8",
    "string": "#cfcfcf",
    "number": "#c2c2c2",
    "type": "#dcdcdc",
    "operator": "#bdbdbd",
    "punctuation": "#9a9a9a"
  },
  "markdown": {
    "heading": "#f0f0f0",
    "link": "#d0d0d0",
    "linkText": "#bdbdbd",
    "code": "#cfcfcf",
    "blockQuote": "#a8a8a8",
    "emph": "#a8a8a8",
    "strong": "#f0f0f0",
    "listItem": "#b8b8b8"
  }
}
```

## OpenCode TUI Format

OpenCode TUI theme JSON files are supported directly. These are the files shaped like `opencode-dev/packages/opencode/src/cli/cmd/tui/context/theme/matrix.json`: colors live in `theme`, optional reusable colors live in `defs`, and `{ "dark": "...", "light": "..." }` variants are resolved using the dark value.

OpenCode TUI files do not include an `id`; Chariox uses the JSON filename. For example, `~/.chariox/themes/neon-matrix.json` appears as `Neon Matrix` in the waiting room.

```json
{
  "$schema": "https://opencode.ai/theme.json",
  "defs": {
    "bg": "#000000",
    "ink": "#62ff94",
    "green": "#2eff6a",
    "dim": "#8ca391"
  },
  "theme": {
    "primary": { "dark": "green", "light": "green" },
    "text": { "dark": "ink", "light": "bg" },
    "textMuted": { "dark": "dim", "light": "dim" },
    "background": { "dark": "bg", "light": "#ffffff" },
    "syntaxKeyword": { "dark": "#c770ff", "light": "#c770ff" },
    "markdownHeading": "primary"
  }
}
```

## OpenCode Desktop Format

OpenCode desktop theme JSON files are also supported. Chariox reads the `dark.palette` and `dark.overrides` fields and adapts them to CLI tokens.

```json
{
  "$schema": "https://opencode.ai/desktop-theme.json",
  "id": "custom-matrix",
  "name": "Custom Matrix",
  "dark": {
    "palette": {
      "neutral": "#000000",
      "ink": "#62ff94",
      "primary": "#2eff6a",
      "accent": "#c770ff",
      "success": "#62ff94",
      "warning": "#e6ff57",
      "error": "#ff4b4b",
      "info": "#30b3ff"
    },
    "overrides": {
      "text-weak": "#8ca391",
      "syntax-keyword": "#c770ff"
    }
  }
}
```

After saving a file, restart the CLI. The theme will appear in the waiting room.
