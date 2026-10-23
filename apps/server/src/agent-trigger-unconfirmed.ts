import type { TriggerDelivery } from "@chariox/kernel-client/kernel-types"

export type Unconfirmed = Pick<TriggerDelivery, "prompt_id" | "origin" | "prompt">

const escape = (text: string) => text.replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!)

/** Firings the provider did not confirm: never sent again by themselves, so the person decides. */
export function unconfirmedHtml(items: readonly Unconfirmed[]): string {
  return items.map(item => `<div class="unconfirmed" data-prompt-id="${escape(item.prompt_id)}"><span><strong>Not confirmed</strong> · ${escape(item.origin === "http" ? "HTTP" : item.origin.replaceAll("_", " "))} · ${escape(item.prompt.split(/\r?\n/, 1)[0] ?? "")}</span><button type="button" data-resend="${escape(item.prompt_id)}" title="Send this message to the agent once more">Resend</button></div>`).join("")
}
