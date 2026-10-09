/** Kernel-issued codes only. Unknown or generic transport errors are never refusals. */
export const userDomainRefusalReasons = ["not_focused_agent", "foreign_owner", "stale_epoch", "stale_reference", "not_granted", "sensitive_requires_focus", "not_requested"] as const
export type UserDomainRefusalReason = typeof userDomainRefusalReasons[number]
export function userDomainRefusalReason(code: unknown): UserDomainRefusalReason | null {
  return userDomainRefusalReasons.find(reason => code === "user_domain_" + reason) ?? null
}
