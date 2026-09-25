/** Terminal-local `/app dev`: pack a source directory, install or update it, and repeat on change. */
import { watch as watchFileSystem } from "node:fs"
import { mkdtemp, realpath, rm, stat } from "node:fs/promises"
import { homedir, tmpdir } from "node:os"
import { isAbsolute, join, relative, resolve, sep } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { listAppInstallationsRequest } from "@chariox/kernel-client/ipc-requests"
import type { AppInstallationSummary, AppInstallOperationSummary } from "@chariox/kernel-client/kernel-types"
import { packAppPackage, type PackedAppPackage } from "./app-developer.js"
import { formatInstallFailure, KernelFailure, terminalCwd, type AppFileInstaller } from "./app-install-file.js"

type Send = (request: Record<string, unknown>) => Promise<Record<string, unknown>>
export type AppDevWatcher = { close(): void }
export type AppDevPackOptions = { bundle: string; manifest: string; key: string; output: string }
export type AppDevDeps = {
  send: Send
  installer: Pick<AppFileInstaller, "install" | "update" | "status" | "cancel" | "retained" | "discardRetained">
  notice: (message: string) => void
  currentSession: () => string | undefined
  pack?: (options: AppDevPackOptions) => Promise<PackedAppPackage>
  watch?: (directory: string, onChange: (file: string | null) => void, onError: (error: unknown) => void) => AppDevWatcher
  /** A new private directory for packed output; it must be outside the App directory. */
  workspace?: () => Promise<string>
  cwd?: string
  home?: string
  debounceMs?: number
  pollMs?: number
}

const activePhases = new Set(["preparing", "starting", "awaiting_approval"])
export const defaultAppDevKey = (home: string) => join(home, ".chariox", "dev", "app-publisher", "private")

/** One dev loop per terminal; starting another stops the previous one. */
export class AppDevLoop {
  private current: DevRun | undefined
  constructor(private deps: AppDevDeps) {}

  get active(): boolean { return this.current !== undefined }

  /** Checks inputs and starts watching; the first cycle runs in the background. */
  async start(directory: string, options: { key?: string } = {}): Promise<{ directory: string; key: string }> {
    const session = this.deps.currentSession()
    if (!session) throw new Error("Attach to a session before starting /app dev")
    const cwd = this.deps.cwd ?? terminalCwd()
    const root = await sourceDirectory(resolve(cwd, directory))
    const key = resolve(cwd, options.key ?? defaultAppDevKey(this.deps.home ?? homedir()))
    if (!(await stat(key).then(entry => entry.isFile(), () => false))) throw new Error(missingKey(key))
    const workspace = await (this.deps.workspace ?? privateWorkspace)()
    if (contains(root, workspace)) {
      await rm(workspace, { recursive: true, force: true })
      throw new Error("The temporary package directory must be outside the App directory")
    }
    await this.stop()
    const run = new DevRun(this.deps, root, key, workspace, session, () => { if (this.current === run) this.current = undefined })
    this.current = run
    run.begin()
    return { directory: root, key }
  }

  /** Stops watching; a cycle already running finishes its current kernel request. */
  async stop(): Promise<boolean> {
    const run = this.current
    this.current = undefined
    if (!run) return false
    await run.close()
    return true
  }

  /** Test and shutdown helper: resolves once no debounce timer or cycle is outstanding. */
  async settled(): Promise<void> { await this.current?.settled() }

  async dispose(): Promise<void> { await this.stop() }
}

class DevRun {
  private watcher: AppDevWatcher | undefined
  private timer: ReturnType<typeof setTimeout> | undefined
  private running: Promise<void> | undefined
  private pending = false
  private stopped = false
  private closing: Promise<void> | undefined
  private abort = new AbortController()
  private suggestedOpen = false
  private readonly output: string

  constructor(private deps: AppDevDeps, private root: string, private key: string, private workspace: string,
    private session: string, private onClosed: () => void) {
    this.output = join(workspace, "app-dev.cxapp")
  }

  begin(): void {
    const watch = this.deps.watch ?? watchDirectory
    try {
      this.watcher = watch(this.root, file => { if (!ignored(file)) this.schedule() }, error => {
        this.deps.notice(`App dev loop stopped: ${message(error)}`)
        void this.close()
      })
    } catch (error) {
      void this.close()
      throw error
    }
    this.deps.notice(`Watching ${this.root} for changes. Use /app dev stop to stop.`)
    this.trigger()
  }

  async settled(): Promise<void> {
    while (this.timer || this.running) await (this.running ?? delay(this.deps.debounceMs ?? 500))
  }

  close(): Promise<void> {
    this.closing ??= (async () => {
      this.stopped = true
      if (this.timer) clearTimeout(this.timer)
      this.timer = undefined
      this.pending = false
      this.watcher?.close()
      this.abort.abort()
      this.onClosed()
      await this.running?.catch(() => {})
      await rm(this.workspace, { recursive: true, force: true }).catch(() => {})
    })()
    return this.closing
  }

  private schedule(): void {
    if (this.stopped) return
    if (this.timer) clearTimeout(this.timer)
    this.timer = setTimeout(() => { this.timer = undefined; this.trigger() }, this.deps.debounceMs ?? 500)
  }

  /** Never overlaps cycles: changes during a cycle coalesce into exactly one more. */
  private trigger(): void {
    if (this.stopped) return
    if (this.running) { this.pending = true; return }
    const cycle = this.cycle().catch(error => {
      if (!this.stopped) this.deps.notice(`App dev: ${message(error)}`)
    }).finally(() => {
      if (this.running === cycle) this.running = undefined
      if (this.pending && !this.stopped) { this.pending = false; this.trigger() }
    })
    this.running = cycle
  }

  private async cycle(): Promise<void> {
    if (this.deps.currentSession() !== this.session) {
      this.deps.notice("App dev loop stopped: this terminal is no longer attached to the session it started in.")
      void this.close()
      return
    }
    try {
      if (!(await this.settleRetained())) return
      await rm(this.output, { force: true })
      const packed = await (this.deps.pack ?? packAppPackage)({
        bundle: join(this.root, "bundle"), manifest: join(this.root, "app.json"), key: this.key, output: this.output,
      })
      if (this.stopped) return
      this.deps.notice(`Packed ${packed.appId} ${packed.version} (${shortDigest(packed.packageDigest)})`)
      const existing = await this.findInstallation(packed.appId)
      if (this.stopped) return
      const started = existing
        ? await this.deps.installer.update(existing, this.output, this.session)
        : await this.deps.installer.install(this.output, this.session)
      const status = await this.follow(started)
      if (status) this.report(status, existing)
    } finally {
      await rm(this.output, { force: true }).catch(() => {})
    }
  }

  /**
   * A previous cycle's attempt retained after a connection failure is settled before packing: a begun operation
   * is followed to its end and reported as the earlier build, an unbegun upload is cancelled. The cycle then
   * uploads the fresh pack, so the latest edit is never answered with the old operation's result.
   */
  private async settleRetained(): Promise<boolean> {
    const retained = this.deps.installer.retained()
    if (!retained || retained.path !== this.output) return true
    const started = retained.begun ? await this.deps.installer.status(retained.request).catch((error: unknown) => {
      if (error instanceof KernelFailure && error.code === "not_found") return undefined
      throw error
    }) : undefined
    if (!started) { await this.deps.installer.cancel(); return !this.stopped }
    const status = await this.follow(started)
    if (!status) return false
    this.report(status, retained.installation, `Previous build${retained.digest ? ` (${shortDigest(retained.digest)})` : ""}: `)
    this.deps.installer.discardRetained()
    return !this.stopped
  }

  private async follow(status: AppInstallOperationSummary): Promise<AppInstallOperationSummary | undefined> {
    let announced = false
    while (activePhases.has(status.phase)) {
      if (status.phase === "awaiting_approval" && !announced) {
        announced = true
        this.deps.notice("Awaiting your approval in the approval panel: this release requests new or changed capabilities.")
      }
      try { await delay(this.deps.pollMs ?? 500, undefined, { signal: this.abort.signal }) } catch { return undefined }
      if (this.stopped) return undefined
      status = await this.deps.installer.status(status.request_id)
    }
    return status
  }

  private report(status: AppInstallOperationSummary, existing: string | undefined, prefix = ""): void {
    const verb = existing ? "Update" : "Install"
    if (status.phase === "failed") {
      this.deps.notice(`${prefix}${verb} failed: ${formatInstallFailure(status.failure ?? "")} Operation ${status.request_id}.`)
      return
    }
    if (status.phase !== "committed") {
      this.deps.notice(`${prefix}${verb} cancelled. Operation ${status.request_id}.`)
      return
    }
    const id = status.installation_id ?? existing ?? "App"
    const generation = status.generation ?? "?"
    if (!existing) this.deps.notice(`${prefix}Installed ${id} at generation ${generation}.`)
    // The loop opens no views; views opened on an older generation must be reopened.
    else this.deps.notice(`${prefix}Updated ${id} to generation ${generation}; App data is kept.${this.suggestedOpen ? ` Reopen views opened earlier with /app open ${id}` : ""}`)
    if (!this.suggestedOpen) this.deps.notice(`Open its view with /app open ${id}`)
    this.suggestedOpen = true
  }

  private async findInstallation(appId: string): Promise<string | undefined> {
    let after: string | undefined
    for (let page = 0; page < 1000; page++) {
      const reply = await this.deps.send(listAppInstallationsRequest({ ...(after ? { after } : {}), limit: 100 }))
      const failure = reply.AppRequestFailed as { code?: unknown } | undefined
      if (failure) throw new Error(`Could not list App installations (${typeof failure.code === "string" ? failure.code : "failed"})`)
      const listed = reply.AppInstallationsListed as { installations?: AppInstallationSummary[]; next_cursor?: string | null } | undefined
      if (!listed || !Array.isArray(listed.installations)) throw new Error("Kernel returned an invalid App installation list")
      // An uninstalled installation keeps its row without an active release; a new install replaces it.
      const match = listed.installations.find(value => value.app_id === appId && value.active_release)
      if (match) return match.installation_id
      if (!listed.next_cursor || listed.next_cursor === after) return undefined
      after = listed.next_cursor
    }
    throw new Error("Too many App installations to search")
  }
}

async function sourceDirectory(selected: string): Promise<string> {
  const root = await realpath(selected).catch(() => { throw new Error(`App directory not found: ${selected}`) })
  const [manifest, bundle] = await Promise.all([
    stat(join(root, "app.json")).then(entry => entry.isFile(), () => false),
    stat(join(root, "bundle")).then(entry => entry.isDirectory(), () => false),
  ])
  if (!manifest || !bundle) throw new Error(`${root} is not an App source directory: expected app.json and bundle/ (create one with chariox app create).`)
  return root
}

function missingKey(key: string): string {
  return [
    `No developer signing key at ${key}.`,
    "Create one once, outside your project, and enroll its publisher:",
    '  mkdir -p "$HOME/.chariox/dev" && mkdir -m 700 "$HOME/.chariox/dev/app-publisher"',
    '  chariox app keygen --publisher-id ID --publisher-name NAME --key-out "$HOME/.chariox/dev/app-publisher/private" --trust-out "$HOME/.chariox/dev/app-publisher/publisher.json"',
    '  /app publisher enroll "$HOME/.chariox/dev/app-publisher/publisher.json"',
    "Or pass --key PRIVATE to use another key.",
  ].join("\n")
}

/** mkdtemp creates the directory 0700; realpath because the packer refuses symlinked components. */
async function privateWorkspace(): Promise<string> {
  return mkdtemp(join(await realpath(tmpdir()), "chariox-app-dev-"))
}

function watchDirectory(directory: string, onChange: (file: string | null) => void, onError: (error: unknown) => void): AppDevWatcher {
  const watcher = watchFileSystem(directory, { recursive: true, persistent: false }, (_event, file) => { onChange(file === null ? null : String(file)) })
  watcher.on("error", onError)
  return watcher
}

/** Dotfiles and dependency trees do not trigger a cycle. */
export function ignored(file: string | null): boolean {
  if (file === null) return false
  return file.split(/[\\/]/).some(part => part.startsWith(".") || part === "node_modules")
}

function contains(root: string, path: string): boolean {
  const inner = relative(root, path)
  return inner === "" || !(inner === ".." || inner.startsWith(`..${sep}`) || isAbsolute(inner))
}

function shortDigest(digest: string): string { return `${digest.slice(0, 19)}…` }
function message(error: unknown): string { return error instanceof Error ? error.message : String(error) }
