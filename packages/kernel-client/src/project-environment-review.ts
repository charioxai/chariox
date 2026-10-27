// MP-08 / MP-10 / MP-11: Value-free projection of the kernel-owned export review.
export const projectEnvironmentReviewMinimumProtocolVersion = 371
export type ProjectEnvironmentReview = {
  readonly schema_version: 1
  readonly project_name: string
  readonly target_name: string
  readonly expanded: boolean
  readonly unattended: boolean
  readonly changed_only: boolean
  readonly rows: readonly {readonly label: string; readonly summary: string}[]
  readonly files: readonly {readonly id: string; readonly path: string; readonly bring: boolean; readonly reason: string; readonly changed: boolean}[]
  readonly inputs: readonly {
    readonly id: string; readonly name: string; readonly workspace_id: string
    readonly missing: boolean; readonly secret: boolean; readonly source: string; readonly changed: boolean
    readonly uses: readonly {readonly path: string; readonly line: number}[]
  }[]
}

export function projectEnvironmentReviewLines(review: ProjectEnvironmentReview): string[] {
  const lines = review.unattended ? ["Saved setup applied"] : []
  lines.push(...review.rows.map(row => `${row.label}: ${row.summary}`))
  const missing = review.inputs.filter(input => input.missing)
  if (missing.length) lines.push(`Needs you: ${missing.map(input => input.name).join(", ")}`)
  if (review.expanded) {
    for (const file of review.files.filter(file => !review.changed_only || file.changed)) {
      lines.push(`${file.bring ? "Bring" : "Leave"} ${file.path} — ${file.reason}${file.changed && review.changed_only ? " (new)" : ""}`)
    }
    for (const input of review.inputs.filter(input => !review.changed_only || input.changed)) {
      lines.push(`${input.name}${input.secret ? " · secret hidden" : ""} — ${input.source}; ${input.uses.map(use => `${use.path}:${use.line}`).join(", ")}`)
    }
  }
  return lines
}
