/** MP-08/MP-10: native terminal selection is the default, including over SSH.
 * App mouse controls and their copy-safe override are an explicit opt-in. */
export const TUI_MOUSE_ENABLED = process.env.CHARIOX_TUI_MOUSE === "on"
