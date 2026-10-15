// MP-08 / MP-10 / MP-11: source-bound chips never submit through the picker.
import { SkillsClient, skillPickerRows, insertSkillChip, reconcileSkillChips, skillTokenAtCursor, type SkillChip, type SkillFocus, type SkillsContext } from "./skills-client.js";
import type { SkillSelection } from "./kernel-types-skills-v2.js";
export class SkillsComposerController {
    readonly client: SkillsClient;
    chips: SkillChip[] = [];
    private text = "";
    private loadedKey = "";
    private loadingKey = "";
    private error = "";
    private draftKey = "";
    private pendingDraft: {
        text: string;
        chips: SkillChip[];
    } | null = null;
    constructor(private readonly deps: {
        draft?: {
            key(): string;
            read(key: string): {
                text: string;
                chips: SkillChip[];
            } | null;
            write(key: string, draft: {
                text: string;
                chips: SkillChip[];
            }): void;
        };
        context(): SkillsContext;
        provider(): string;
        read(): {
            text: string;
            cursor: number;
        };
        write(text: string, cursor: number): void;
        send(request: unknown): Promise<unknown>;
        changed(): void;
        render(chips: SkillChip[], remove: (index: number) => void): void;
    }) { this.client = new SkillsClient(request => deps.send(request)); }
    private focus(): SkillFocus | null {
        if (!this.deps.context().agentId)
            return null;
        const selection = this.client.catalog?.native.map(n => n.selection).find(s => s.source === "native" && s.provider === this.deps.provider());
        return selection?.source === "native" ? { provider: selection.provider, account: selection.account_profile, execution: selection.execution_id } : null;
    }
    sync(text: string): void {
        const draftKey = this.deps.draft?.key() ?? "";
        if (draftKey !== this.draftKey) {
            this.draftKey = draftKey;
            this.pendingDraft = draftKey ? this.deps.draft?.read(draftKey) ?? null : null;
            this.text = text;
            this.chips = [];
        }
        if (this.pendingDraft) {
            if (this.pendingDraft.text === text) {
                this.chips = this.pendingDraft.chips;
                this.pendingDraft = null;
                this.text = text;
            }
            else if (this.text !== text)
                this.pendingDraft = null;
        }
        const context = this.deps.context();
        if (this.loadedKey !== context.key && this.client.key !== context.key) {
            this.chips = this.chips.map(c => c.selection.source === "native" ? { ...c, invalid: "Reselect this skill for the focused agent and account." } : c);
            this.client.invalidate(context.key);
            this.error = "";
        }
        if (text !== this.text) {
            let prefix = 0;
            while (prefix < text.length && prefix < this.text.length && text[prefix] === this.text[prefix])
                prefix++;
            let suffix = 0;
            while (suffix < text.length - prefix && suffix < this.text.length - prefix && text[text.length - 1 - suffix] === this.text[this.text.length - 1 - suffix])
                suffix++;
            const oldEnd = this.text.length - suffix;
            const delta = text.length - this.text.length;
            this.chips = this.chips.flatMap(c => c.end <= prefix ? [c] : c.start >= oldEnd ? [{ ...c, start: c.start + delta, end: c.end + delta }] : []);
        }
        this.text = text;
        this.chips = reconcileSkillChips(this.chips, text, this.client.catalog?.native ?? [], this.focus());
        this.render();
        const token = skillTokenAtCursor(text, this.deps.read().cursor);
        if (token && this.loadedKey !== context.key && this.loadingKey !== context.key)
            void this.refresh(context);
    }
    private async refresh(context: SkillsContext): Promise<void> {
        this.loadingKey = context.key;
        try {
            await this.client.refresh(context);
            if (this.deps.context().key === context.key) {
                this.loadedKey = context.key;
                this.error = "";
            }
        }
        catch (e) {
            if (this.deps.context().key === context.key) {
                this.loadedKey = context.key;
                this.error = e instanceof Error ? e.message : "Skills unavailable";
            }
        }
        finally {
            if (this.loadingKey === context.key)
                this.loadingKey = "";
            this.deps.changed();
        }
    }
    items() {
        const { text, cursor } = this.deps.read();
        const token = skillTokenAtCursor(text, cursor);
        if (!token)
            return null;
        const context = this.deps.context();
        const rows = skillPickerRows(this.client.catalog?.skills ?? [], this.client.catalog?.native ?? [], this.client.catalog?.revision ?? 0, this.focus(), token.query);
        return { rows, guidance: this.error || (!context.agentId ? "Choose an agent to see its provider skills. Chariox skills stay available." : this.loadingKey ? "Loading skills…" : rows.length ? "" : "No selectable skill. Open /skills to install or enable it.") };
    }
    select(selection: SkillSelection): void {
        const { text, cursor } = this.deps.read();
        const token = skillTokenAtCursor(text, cursor);
        if (!token)
            return;
        const result = insertSkillChip(text, token, selection);
        const delta = result.text.length - text.length;
        this.chips = this.chips.filter(c => c.end <= token.start || c.start >= token.end).map(c => c.start >= token.end ? { ...c, start: c.start + delta, end: c.end + delta } : c);
        this.chips.push(result.chip);
        this.text = result.text;
        this.deps.write(result.text, result.cursor);
        this.render();
    }
    invalidate(): void { this.loadedKey = ""; this.client.invalidate(); this.error = ""; }
    submission(text: string, agentId: string) {
        const selections = this.selections(text, agentId);
        const chips = this.chips.map(chip => ({ ...chip }));
        return { selections, restore: () => this.restore(text, chips) };
    }
    restore(text: string, chips: readonly SkillChip[]): void {
        this.text = text;
        this.chips = chips.map(chip => ({ ...chip }));
        this.sync(text);
    }
    selections(text: string, agentId: string): SkillSelection[] | null {
        this.sync(text);
        if (this.pendingDraft?.chips.length)
            throw new Error("Skill sources are still restoring. Restore the matching draft or select the skill again.");
        if (this.chips.some(c => c.invalid))
            throw new Error("Reselect or remove stale provider skill chips before sending.");
        if (this.chips.some(c => c.selection.source === "native") && agentId !== this.deps.context().agentId)
            throw new Error("Reselect provider skills for the addressed agent.");
        return this.chips.length || text.includes("$") ? this.chips.map(c => c.selection) : null;
    }
    clear(): void { this.chips = []; this.text = ""; this.render(); }
    private render(): void {
        if (this.draftKey && !this.pendingDraft)
            this.deps.draft?.write(this.draftKey, { text: this.text, chips: this.chips });
        this.deps.render(this.chips, index => {
            const chip = this.chips[index];
            if (!chip)
                return;
            const next = this.text.slice(0, chip.start) + this.text.slice(chip.end);
            this.sync(next);
            this.deps.write(next, chip.start);
        });
    }
}
