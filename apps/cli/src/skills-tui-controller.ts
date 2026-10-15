// MP-08 / MP-10 / MP-11: local and remote TUI share the home Skills v2 authority.
import { SkillsClient, skillGroups, providerLabel, type SkillsContext } from "@chariox/kernel-client/skills-client";
import type { SkillCatalogAction, SkillCatalogMutation, SkillInstallSource, SkillReviewReceipt, SkillScope, SkillState } from "@chariox/kernel-client/kernel-types-skills-v2";
export class SkillsTuiController {
    readonly client: SkillsClient;
    private scope: "chariox" | "project" = "chariox";
    private review: {
        key: string;
        mutation: SkillCatalogMutation;
        receipt: SkillReviewReceipt;
    } | null = null;
    constructor(private readonly deps: {
        context(): SkillsContext;
        provider(): string;
        notice(text: string): void;
        send(request: any): Promise<unknown>;
    }) { this.client = new SkillsClient(r => deps.send(r)); }
    async command(args: string[]): Promise<void> {
        const [command = "list", ...rest] = args;
        const context = this.deps.context();
        if (command === "confirm") {
            if (!this.review || this.review.key !== context.key) {
                this.review = null;
                throw new Error("No current Skills review. Repeat the action for this kernel and Project.");
            }
            await this.client.publish(this.review.mutation, this.review.receipt);
            this.review = null;
            await this.refresh();
            this.deps.notice("Skills published. Applicable agents see this revision next turn.");
            return;
        }
        if (command === "cancel") {
            this.review = null;
            this.deps.notice("Skills review cancelled");
            return;
        }
        if (command === "chariox" || command === "project") {
            if (command === "project" && !context.projectId)
                throw new Error("Choose or create a Project; it does not need folders.");
            this.scope = command;
            this.review = null;
            await this.refresh();
            this.list();
            return;
        }
        await this.refresh();
        if (command === "list" || command === "ls" || command === "refresh") {
            this.list();
            return;
        }
        if (command === "from") {
            this.native();
            return;
        }
        if (command === "environment") {
            if (!context.projectId)
                throw new Error("Choose a Project to view Environment Skills");
            this.scope = "project";
            if(rest.length){await this.command(rest);return;}
            this.list();
            this.dependencies();
            return;
        }
        if (command === "show") {
            const skill = this.find(rest[0]);
            const result = await this.client.request({ action: "get", skill_id: skill.skill.id, project_id: context.projectId });
            if (result.kind !== "detail")
                throw new Error("Skill details unavailable");
            this.deps.notice(`${result.skill.skill.name}\n${result.skill.skill.digest}\n${result.skill.skill.files.map(f => `${f.path} · ${f.bytes} bytes · ${f.executable ? "Executable" : "File"} · ${f.sha256}`).join("\n")}\n${result.skill.skill.compatibility_notes.join("\n")}\n${result.body}`);
            return;
        }
        if (command === "show-from") {
            const entry = this.nativeEntry(rest[0]);
            const result = await this.client.nativeDetails(context.agentId!, entry.selection);
            if (context.key !== this.deps.context().key)
                throw new Error("Focused provider changed. Refresh /skills from and select again.");
            const d = result.details;
            this.deps.notice(`${d.entry.selection.name} · From ${providerLabel(this.deps.provider())} · read-only\n${d.declared_version ?? "No declared version"} · ${d.entry.digest ?? "Digest unverified"}\n${d.provenance.locator ?? "Source unavailable"} · ${d.provenance.source_machine ?? "Unknown machine"} · ${d.provenance.provider_version ?? "Unknown provider version"}\n${d.entry.complete_manifest ? "Complete file manifest" : "File manifest incomplete"}\n${d.files.map(f => `${f.path} · ${f.bytes} bytes · ${f.executable ? "Executable" : "File"} · ${f.sha256}`).join("\n")}\n${d.compatibility_notes.join("\n")}\n${d.unavailable_reason ?? ""}\n${d.body ?? "Body unavailable; no source copied or imported."}`);
            return;
        }
        const scope: SkillScope = this.scope === "project" ? { kind: "project", project_id: context.projectId ?? "" } : { kind: "chariox" };
        if (scope.kind === "project" && !scope.project_id)
            throw new Error("Choose a Project first");
        let actions: SkillCatalogAction[];
        if (command === "install") {
            const [kind, value, ...options] = rest;
            const source = this.source(kind, value);
            const index = options.findIndex(s => s === "rename" || s === "replace");
            const collision = index < 0 ? { action: "reject" as const } : options[index] === "rename" ? { action: "rename" as const, name: options[index + 1] ?? "" } : { action: "replace" as const, skill_id: this.find(options[index + 1]).skill.id };
            actions = [{ action: "install", source, collision, package_id: null }];
        }
        else if (command === "import") {
            const entry = this.nativeEntry(rest[0]);
            actions = [{ action: "install", source: { kind: "native", selection: entry.selection }, collision: { action: "reject" }, package_id: null }];
        }
        else if (command === "state") {
            actions = [{ action: "set_state", skill_id: this.find(rest[0]).skill.id, state: this.state(rest[1]) }];
        }
        else if (command === "remove") {
            actions = [{ action: "remove", skill_id: this.find(rest[0]).skill.id }];
        }
        else if (command === "update") {
            const skill = this.find(rest[0]);
            actions = [{ action: "update", skill_id: skill.skill.id, expected_digest: skill.skill.digest, source: rest[1] ? this.source(rest[1], rest[2]) : this.updateSource(skill.skill.provenance) }];
        }
        else if (command === "create-package") {
            if (!rest.length)
                throw new Error("/skills create-package <name>");
            actions = [{ action: "create_package", package_id: crypto.randomUUID(), name: rest.join(" ") }];
        }
        else if (command === "move-to-package" || command === "ungroup") {
            actions = [{ action: "regroup", skill_id: this.find(rest[0]).skill.id, package_id: command === "ungroup" ? null : this.package(rest[1]).id }];
        }
        else if (command === "package-state") {
            actions = [{ action: "set_package_state", package_id: this.package(rest[0]).id, state: this.state(rest[1]) }];
        }
        else if (command === "remove-package" || command === "ungroup-package") {
            actions = [{ action: "remove_package", package_id: this.package(rest[0]).id, remove_skills: command === "remove-package" }];
        }
        else if (command === "update-package") {
            const pkg = this.package(rest[0]);
            actions = (this.client.catalog?.skills ?? []).filter(p => p.skill.package_id === pkg.id).map(p => ({ action: "update", skill_id: p.skill.id, expected_digest: p.skill.digest, source: this.updateSource(p.skill.provenance) }));
            if (!actions.length)
                throw new Error("Package has no members to update");
        }
        else
            throw new Error("/skills [chariox|project|from|show-from <entry>|environment|show <name>|install folder|url|content <source> [rename <name>|replace <id>]|import <entry>|state <name> auto|manual|off|reset|update <name> [folder|url <source>]|remove <name>|create-package <name>|move-to-package <skill> <package>|ungroup <skill>|package-state <package> auto|manual|off|reset|update-package <package>|remove-package <package>|ungroup-package <package>|confirm|cancel]");
        const mutation = { scope, expected_revision: this.client.catalog!.revision, actions };
        const preview = await this.client.preview(mutation);
        if (context.key !== this.deps.context().key)
            throw new Error("Skills target changed. Repeat the action.");
        this.review = { key: context.key, mutation, receipt: preview.review_receipt };
        this.deps.notice(`Review Skills revision ${mutation.expected_revision}\n${actions.map(a => a.action).join("\n")}\nAffected: ${preview.affected.map(s => s.name).join(", ") || "new records"}\nReviewed sources:\n${preview.review_receipt.sources.map(s => `${s.skill_id} · ${s.digest} · ${s.source_revision ?? "Content snapshot"}`).join("\n")}\nReview expires ${new Date(preview.review_receipt.expires_at_ms).toISOString()}\n/skills confirm — publish this batch after kernel authorization\n/skills cancel — discard`);
    }
    private async refresh() { await this.client.refresh(this.deps.context()); }
    private list() {
        const catalog = this.client.catalog!;
        const lines = [`Skills · ${this.scope === "chariox" ? "Chariox" : "Project"} · revision ${catalog.revision} · metadata budget ${catalog.metadata_budget_percent}%`];
        for (const group of skillGroups(catalog.skills.filter(p => p.skill.scope.kind === this.scope), catalog.packages.filter(p => p.scope.kind === this.scope))) {
            lines.push(`${group.name} · ${group.state ?? "Empty"}${group.package ? ` · ${group.package.source_locator ?? "Custom"} · ${group.package.source_revision ?? ""}` : ""}`);
            for (const p of group.members)
                lines.push(`  $${p.skill.name} [${p.effective_state}] ${p.project_override ? "Project override" : p.skill.state_override ? "Skill override" : "Inherited"} · ${p.skill.declared_version ?? p.skill.digest}\n    ${p.skill.description}\n    ${p.skill.provenance.locator ?? p.skill.provenance.kind}${p.also_in_provider ? ` · also in ${providerLabel(p.also_in_provider)}` : ""}${p.delivery.kind === "conflict" ? ` · Conflict: From ${providerLabel(p.delivery.provider)}, Chariox wins` : ""}\n    ${this.client.loads.some(l => l.skill_id === p.skill.id) ? "Loaded in session; history retained" : "Not loaded in session"}`);
        }
        lines.push("/skills from · focused provider, read-only + Import", "/skills show <name> · source, files and body", "/skills state <name> auto|manual|off|reset", "/skills create-package <name> · move-to-package · ungroup", "Type $ at the composer cursor to select a skill without submitting.");
        this.deps.notice(lines.join("\n"));
    }
    private native() { if (!this.deps.context().agentId) {
        this.deps.notice("Choose an agent to see skills From its provider. Chariox skills remain available.");
        return;
    } this.deps.notice(`From ${providerLabel(this.deps.provider())} · read-only\n${this.client.catalog?.native.filter(n => n.selection.source === "native" && n.selection.provider === this.deps.provider()).map(n => `${n.selection.name} · ${n.exposure} · ${n.digest ?? "Unverified digest"}\n${n.description}\n/skills show-from ${n.selection.source === "native" ? n.selection.entry_id : ""}\n/skills import ${n.selection.source === "native" ? n.selection.entry_id : ""}`).join("\n") || "No provider skills"}`); }
    private nativeEntry(name?: string) {
        const entry = this.client.catalog?.native.find(n => n.selection.source === "native" && n.selection.provider === this.deps.provider() && (n.selection.entry_id === name || n.selection.name === name));
        if (!this.deps.context().agentId || !entry)
            throw new Error("Focus an agent and choose an entry from /skills from");
        return entry;
    }
    private dependencies() { this.deps.notice(`Chariox dependencies · Project overrides\n${this.client.catalog?.skills.filter(p => p.skill.scope.kind === "chariox").map(p => `${p.skill.name} · ${p.effective_state} · ${p.project_override ?? "Inherited"} · ${p.skill.digest}`).join("\n")}`); }
    private find(name?: string) { const skill = this.client.catalog?.skills.find(p => p.skill.id === name || p.skill.name === name); if (!skill)
        throw new Error("Skill unavailable. Refresh /skills and choose its name or ID."); return skill; }
    private package(name?: string) { const pkg = this.client.catalog?.packages.find(p => p.id === name || p.name === name); if (!pkg)
        throw new Error("Choose a package from /skills"); return pkg; }
    private state(value?: string): SkillState | null { if (value === "reset")
        return null; if (value === "auto" || value === "manual" || value === "off")
        return value; throw new Error("Choose auto, manual, off or reset"); }
    private source(kind?: string, value?: string): SkillInstallSource { if (!value)
        throw new Error("/skills install folder|url|content <source>"); if (kind === "folder")
        return { kind: "folder", path: value }; if (kind === "url")
        return { kind: "url", url: value, pinned_revision: null }; if (kind === "content")
        {
        const body: unknown = JSON.parse(value);
        if (typeof body !== "string") throw new Error("Skill content must be a JSON-encoded string");
        return { kind: "content", skill_md: body };
    } throw new Error("Choose folder, url or JSON-encoded content"); }
    private updateSource(p: {
        kind: string;
        locator: string | null;
    }): SkillInstallSource { if (!p.locator)
        throw new Error("No update source. Supply folder or url explicitly."); return this.source(p.kind === "folder" ? "folder" : "url", p.locator); }
}
