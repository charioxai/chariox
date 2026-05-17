export function formatToolStatusBadge(status?: string | null) {
  switch (status) {
    case "running":
      return " · RUNNING"
    case "completed":
      return " · COMPLETED"
    case "error":
      return " · ERROR"
    case "cancelled":
      return " · CANCELLED"
    default:
      return status ? ` · ${status.trim().toUpperCase()}` : ""
  }
}
