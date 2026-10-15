export { skillsRuntimeAvailable, requireSkillsRuntime } from "./skills-runtime-admission.js";
// MP-08 / MP-10 / MP-11: presentation and typed client state; kernel owns resolution.
import type { NativeSkillEntry, SkillCatalogMutation, SkillLoadRecord, SkillPackageRecord, SkillProjection, SkillReviewReceipt, SkillSelection, SkillsV2Request, SkillsV2Response } from "./kernel-types-skills-v2.js";
export type SkillCatalogView = Extract<SkillsV2Response, {
    kind: "catalog";
}>;
export type SkillFocus = {
    provider: string;
    account: string;
    execution: string;
};
export type SkillPickerRow = {
    id: string;
    label: string;
    description: string;
    selection: SkillSelection;
};
export type SkillChip = {
    start: number;
    end: number;
    selection: SkillSelection;
    invalid: string | null;
};
export type SkillToken = {
    start: number;
    end: number;
    query: string;
};
const providerLabels: Record<string, string> = { codex: "Codex", claude: "Claude", opencode: "OpenCode" };
export const providerLabel = (provider: string): string => providerLabels[provider] ?? provider;
export function hasSkillReferences(text: string): boolean {
    let fence: { marker: string; length: number } | null = null;
    let span = 0;
    const whitespace = (char: string) => /[ \t\r\n\f\v]/.test(char);
    for (const line of text.match(/[^\n]*\n|[^\n]+$/g) ?? []) {
        const indent = /^ */.exec(line)![0].length;
        const block = /^(`{3,}|~{3,})/.exec(line.slice(indent));
        if (!span && indent <= 3 && block) {
            const marker = block[1]![0]!;
            const length = block[1]!.length;
            if (!fence) fence = { marker, length };
            else if (fence.marker === marker && length >= fence.length && !line.slice(indent + length).trim()) fence = null;
            continue;
        }
        if (fence || (indent >= 4 && !span)) continue;
        for (let i = 0; i < line.length;) {
            const char = line[i]!;
            if (span) {
                if (char === "`") {
                    let end = i + 1;
                    while (line[end] === "`") end++;
                    if (end - i === span) span = 0;
                    i = end;
                } else i++;
                continue;
            }
            if (char === "[") {
                const close = line.indexOf("](", i);
                const end = close < 0 ? -1 : line.indexOf(")", close + 2);
                if (end >= 0) { i = end + 1; continue; }
            }
            if (char === "\\") { i += 2; continue; }
            if (char === "`") {
                const start = i++;
                while (line[i] === "`") i++;
                span = i - start;
                continue;
            }
            if ((char === "'" || char === '"') && !(i > 0 && /[a-zA-Z0-9]/.test(line[i - 1]!))) {
                i++;
                while (i < line.length) {
                    if (line[i] === "\\") i += 2;
                    else if (line[i++] === char) break;
                }
                continue;
            }
            if (char !== "$") { i++; continue; }
            const start = i++;
            if (start > 0 && !whitespace(line[start - 1]!) && !"([".includes(line[start - 1]!)) continue;
            const nameStart = i;
            while (i < line.length && /[a-zA-Z0-9_-]/.test(line[i]!)) i++;
            const name = line.slice(nameStart, i);
            if (!/^[a-zA-Z]/.test(name) || name.length > 128 || /^[A-Z0-9_]+$/.test(name)) continue;
            if (i < line.length && !whitespace(line[i]!) && !")],.!?;:".includes(line[i]!)) continue;
            if (line[i] === "]" && line[i + 1] === "(") continue;
            return true;
        }
    }
    return false;
}
export function skillPickerRows(skills: readonly SkillProjection[], native: readonly NativeSkillEntry[], revision: number, focus: SkillFocus | null, query = ""): SkillPickerRow[] {
    const duplicate = (entry: NativeSkillEntry) => {
        const selection = entry.selection;
        return selection.source === "native" && skills.some(p => p.effective_state !== "off"
            && p.also_in_provider === selection.provider && p.skill.name.toLowerCase() === selection.name.toLowerCase()
            && p.skill.digest === entry.digest);
    };
    return [
        ...skills.filter(p => p.effective_state !== "off").map(p => ({
            id: `chariox:${p.skill.id}`, label: `$${p.skill.name} · ${p.skill.scope.kind === "project" ? "Project" : "Chariox"}${p.also_in_provider ? ` · also in ${providerLabel(p.also_in_provider)}` : ""}`,
            description: p.skill.short_description || p.skill.description,
            selection: { source: "chariox", skill_id: p.skill.id, name: p.skill.name, catalog_revision: revision } as SkillSelection,
        })),
        ...native.filter(n => n.selection.source === "native" && focus && n.selection.provider === focus.provider && n.selection.account_profile === focus.account && n.selection.execution_id === focus.execution && n.exposure !== "disabled" && !duplicate(n))
            .map(n => ({ id: `native:${n.selection.source === "native" ? n.selection.entry_id : ""}`, label: `$${n.selection.name} · From ${providerLabel(n.selection.source === "native" ? n.selection.provider : "")}`, description: n.description, selection: n.selection })),
    ].filter(r => `${r.label} ${r.description}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
}
export function skillGroups(skills: readonly SkillProjection[], packages: readonly SkillPackageRecord[]) {
    const group = (id: string | null, name: string, pkg: SkillPackageRecord | null) => {
        const members = skills.filter(p => p.skill.package_id === id);
        const states = new Set(members.map(p => p.effective_state));
        return { id, name, package: pkg, members, state: states.size > 1 ? "mixed" : members[0]?.effective_state ?? null };
    };
    return [...packages.map(p => group(p.id, p.name, p)), group(null, "Ungrouped", null)];
}
export function skillTokenAtCursor(text: string, cursor: number): SkillToken | null {
    if (!Number.isInteger(cursor) || cursor < 0 || cursor > text.length)
        return null;
    const prefix = text.slice(0, cursor);
    const match = /(?:^|\s)\$([\p{L}\p{N}_-]*)$/u.exec(prefix);
    if (!match || /^[A-Z_][A-Z0-9_]*$/.test(match[1]!) || /^\d/.test(match[1]!))
        return null;
    const start = cursor - match[1]!.length - 1;
    if (insideCode(text.slice(0, start)))
        return null;
    let end = cursor;
    while (end < text.length && /[\p{L}\p{N}_-]/u.test(text[end]!))
        end++;
    return { start, end, query: match[1]! };
}
function insideCode(text: string): boolean {
    let fence: {
        marker: string;
        length: number;
    } | null = null;
    let inline = 0;
    for (const line of text.split("\n")) {
        const block = /^ {0,3}(`{3,}|~{3,})/.exec(line);
        if (block) {
            const marker = block[1]![0]!;
            const length = block[1]!.length;
            if (!fence)
                fence = { marker, length };
            else if (fence.marker === marker && length >= fence.length)
                fence = null;
            continue;
        }
        if (fence)
            continue;
        for (let i = 0; i < line.length; i++) {
            if (line[i] === "\\") {
                i++;
                continue;
            }
            if (line[i] !== "`")
                continue;
            let end = i + 1;
            while (line[end] === "`")
                end++;
            const length = end - i;
            if (inline === 0)
                inline = length;
            else if (inline === length)
                inline = 0;
            i = end - 1;
        }
    }
    return fence !== null || inline !== 0;
}
export function insertSkillChip(text: string, token: SkillToken, selection: SkillSelection) {
    const value = `$${selection.name}`;
    const suffix = text.slice(token.end);
    return { text: text.slice(0, token.start) + value + (suffix.startsWith(" ") ? "" : " ") + suffix,
        cursor: token.start + value.length + 1, chip: { start: token.start, end: token.start + value.length, selection, invalid: null } as SkillChip };
}
export function reconcileSkillChips(chips: readonly SkillChip[], text: string, native: readonly NativeSkillEntry[], focus: SkillFocus | null): SkillChip[] {
    return chips.filter(c => text.slice(c.start, c.end) === `$${c.selection.name}`).map(c => {
        const s = c.selection;
        const valid = s.source === "chariox" || focus && s.provider === focus.provider && s.account_profile === focus.account && s.execution_id === focus.execution && native.some(n => JSON.stringify(n.selection) === JSON.stringify(s) && n.exposure !== "disabled");
        return { ...c, invalid: c.invalid ?? (valid ? null : "Reselect this skill for the focused agent and account.") };
    });
}
export type SkillsContext = {
    key: string;
    projectId: string | null;
    agentId: string | null;
    sessionId: string | null;
};
export class SkillsClient {
    catalog: SkillCatalogView | null = null;
    loads: SkillLoadRecord[] = [];
    key = "";
    private generation = 0;
    constructor(private readonly send: (request: {
        SkillsV2: SkillsV2Request;
    }) => Promise<unknown>) { }
    invalidate(key = this.key): void { this.key = key; this.generation++; this.catalog = null; this.loads = []; }
    async request(request: SkillsV2Request): Promise<SkillsV2Response> {
        const response = await this.send({ SkillsV2: request }) as {
            SkillsV2?: {
                result?: SkillsV2Response;
            };
            Error?: {
                message?: string;
            };
        };
        if (response.Error)
            throw new Error(response.Error.message || "Skills request refused");
        if (!response.SkillsV2?.result)
            throw new Error("Kernel did not return Skills v2. Upgrade or reconnect this kernel.");
        return response.SkillsV2.result;
    }
    async refresh(context: SkillsContext): Promise<void> {
        this.invalidate(context.key);
        const generation = this.generation;
        const result = await this.request({ action: "catalog", project_id: context.projectId, agent_id: context.agentId });
        if (generation !== this.generation)
            return;
        if (result.kind !== "catalog")
            throw new Error("Kernel did not return a skills catalog");
        if (context.sessionId) {
            const loads = await this.request({ action: "loads", session_id: context.sessionId, agent_id: context.agentId });
            if (generation !== this.generation)
                return;
            if (loads.kind !== "loads")
                throw new Error("Kernel did not return skill load history");
            this.loads = loads.records;
        }
        this.catalog = result;
    }
    async preview(mutation: SkillCatalogMutation) {
        const result = await this.request({ action: "preview", mutation });
        if (result.kind !== "preview")
            throw new Error("Kernel did not return the affected skills preview");
        if (result.expected_revision !== mutation.expected_revision)
            throw new Error("Skills preview revision changed. Refresh and repeat the action.");
        if (result.collisions.length)
            throw new Error(`${result.collisions.join("; ")}. Rename or explicitly replace the installed skill.`);
        this.requireReview(result.review_receipt);
        return result;
    }
    async publish(mutation: SkillCatalogMutation, receipt: SkillReviewReceipt): Promise<void> {
        this.requireReview(receipt);
        const result = await this.request({ action: "mutate", mutation, review_receipt_id: receipt.id });
        if (result.kind !== "published")
            throw new Error("Kernel did not publish the skills mutation");
    }
    private requireReview(receipt: SkillReviewReceipt | undefined): void {
        if (!receipt?.id?.trim() || !receipt.mutation_sha256?.trim() || !Number.isFinite(receipt.expires_at_ms) || receipt.expires_at_ms <= Date.now())
            throw new Error("Review the changes again: the home review receipt is missing or expired.");
    }
    async nativeDetails(agentId: string, selection: SkillSelection) {
        if (!agentId || selection.source !== "native")
            throw new Error("Choose a skill From the focused provider.");
        const result = await this.request({ action: "native_details", agent_id: agentId, selection });
        if (result.kind !== "native_details")
            throw new Error("Provider skill details unavailable");
        const actual = result.details.entry.selection;
        if (actual.source !== "native" || actual.entry_id !== selection.entry_id || actual.provider !== selection.provider
            || actual.account_profile !== selection.account_profile || actual.execution_id !== selection.execution_id
            || actual.inventory_revision !== selection.inventory_revision || actual.name !== selection.name)
            throw new Error("Provider skill source changed. Refresh and choose its entry again.");
        return result;
    }
}
