// MP-07 / MP-08 / MP-11: owner-managed SSH installation, protocol 444.
export type SshMachineOptions = { install_id?: string; port?: number; release?: string }
export function addSshMachineRequest(host: string, options: SshMachineOptions = {}) {
  return { AddSshMachine: { host, ...options } }
}
export function removeSshMachineRequest(installId: string) {
  return { RemoveSshMachine: { install_id: installId } }
}
