export function listAppInstallationsRequest(options: { after?: string; limit?: number } = {}) {
  return { ListAppInstallations: { after: options.after ?? null, limit: options.limit ?? null } }
}

export function getAppInstallationRequest(installationId: string) {
  return { GetAppInstallation: { installation_id: installationId } }
}

export function getAppInstallationJournalRequest(installationId: string) {
  return { GetAppInstallationJournal: { installation_id: installationId } }
}
