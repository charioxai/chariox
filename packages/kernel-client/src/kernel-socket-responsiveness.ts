// MP-08/MP-10/MP-11: transport liveness is separate from one request's latency.
export class KernelSocketResponsiveness {
  private readonly pongs = new WeakMap<object, number>()

  recordPong(socket: object): void {
    this.pongs.set(socket, Date.now())
  }

  isResponsive(socket: object, heartbeatHorizonMs: number): boolean {
    const at = this.pongs.get(socket)
    const age = at === undefined ? Infinity : Date.now() - at
    return age >= 0 && age <= heartbeatHorizonMs
  }
}
