import { userAppViewsPrototypeEnabled } from "./user-app-views-flag.js"
/**
 * Every `app` command of the standalone CLI (`chariox app …`) and the TUI
 * (`/app …`): the source of both help texts and of the parity test. A command
 * with only `cli` or only `tui` exists on that surface alone.
 */
export type AppCommandEntry = { verb: string; cli?: string; tui?: string; summary: string }

const both = (verb: string, args: string, summary: string): AppCommandEntry => {
  const usage = `${verb}${args ? ` ${args}` : ""}`
  return { verb, cli: usage, tui: usage, summary }
}

export const appCommandCatalog: readonly AppCommandEntry[] = [
  { verb: "create", cli: "create [options]", summary: "create an App source directory" },
  { verb: "keygen", cli: "keygen [options]", summary: "create a publisher key and its public trust file" },
  { verb: "manifest", cli: "manifest [options]", summary: "check an App manifest" },
  { verb: "validate", cli: "validate [options]", summary: "validate an App source directory" },
  { verb: "pack", cli: "pack [options]", summary: "sign and pack an App into a .cxapp" },
  { verb: "inspect", cli: "inspect [options]", summary: "inspect a packed App" },
  { verb: "dev", tui: 'dev "DIRECTORY" [--key PRIVATE] | dev stop', summary: "pack, install and reload an App source directory on change" },
  { verb: "publisher", tui: 'publisher enroll "publisher.json" [--revision N] | publisher status|cancel [REVIEW-ID]', summary: "enroll an App publisher" },
  { verb: "install", cli: "install FILE.cxapp --session SESSION", tui: 'install "FILE.cxapp"', summary: "install an App; approve it in the session's terminal" },
  { verb: "update", cli: "update INSTALLATION FILE.cxapp --session SESSION", tui: 'update INSTALLATION "FILE.cxapp"', summary: "update an App; approve it in the session's terminal" },
  { verb: "operation", tui: "operation [REQUEST-ID]", summary: "show an App install or update" },
  { verb: "cancel", tui: "cancel [REQUEST-ID]", summary: "cancel an App install or update" },
  both("list", "[--after ID] [--limit 1..100]", "list installed Apps"),
  both("set", "", "show your App set: releases, capabilities and configuration"),
  both("status", "INSTALLATION", "show an App installation"),
  both("journal", "INSTALLATION", "show retained App updates"),
  both("logs", "INSTALLATION [--after SEQUENCE]", "show an App's log"),
  both("worker", "INSTALLATION", "show an App's worker"),
  both("start", "INSTALLATION", "start an App's worker"),
  both("stop", "INSTALLATION", "stop an App's worker"),
  both("restart", "INSTALLATION", "restart an App's worker"),
  { verb: "open", cli: "open INSTALLATION --session SESSION", tui: "open INSTALLATION [--session SESSION]", summary: "open an App's view" },
  both("uninstall", "INSTALLATION [--generation N] [--delete-data]", "uninstall an App"),
  both("automation", "list|add|disable INSTALLATION …", "manage an App's automations"),
  both("inbox", "list|add|remove|test INSTALLATION …", "manage an App's inbox routes"),
  both("connection", "list|grant|revoke INSTALLATION …", "manage an App's connections"),
  { verb: "file", cli: "file revoke INSTALLATION [OPERATION]", tui: 'file grant OPERATION "FILE"… | file save OPERATION "PATH" | file revoke INSTALLATION [OPERATION]', summary: "answer or revoke an App's file request" },
]

export const cliOnlyAppVerbs = new Set(appCommandCatalog.filter((entry) => !entry.tui).map((entry) => entry.verb))
const tuiOnlyAppVerbs = new Set(appCommandCatalog.filter((entry) => !entry.cli).map((entry) => entry.verb))

export function isTuiOnlyAppCommand(verb: string, subcommand?: string): boolean {
  return tuiOnlyAppVerbs.has(verb) || (verb === "file" && subcommand !== "revoke")
}

/** The `chariox app` line of the CLI usage. */
export function cliAppUsage(): string {
  const verbs = appCommandCatalog.filter((entry) => entry.cli).map((entry) => entry.verb)
  return `       chariox app ${verbs.join("|")} [args] [kernel or relay connection options]`
}

/** The `/app` lines of the TUI command help. */
export function tuiAppHelp(enabled = userAppViewsPrototypeEnabled()): string[] {
  return [...appCommandCatalog
    .filter((entry) => entry.tui)
    .map((entry) => `  /app ${entry.tui}  ${entry.summary}`),
    ...(enabled ? ["  /app views | view open INSTALLATION | view show VIEW | view close [VIEW]", "  /app view call METHOD \'JSON\' | view approvals | view answer INTERACTION CHOICE"] : []),
  ]
}
