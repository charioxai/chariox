import { sanitizeDrillMetadata } from "./drill-secrets.mjs"
import {
  DRILL_CHAOS_REPLAY_SCHEMA,
  validateDrillChaosFaultPlan,
  validateDrillChaosReplayBundle,
} from "./drill-chaos-contract.mjs"

export class DrillChaosRandom {
  constructor(seed) {
    this.seed = String(seed)
    this.state = hashSeed(this.seed)
  }

  nextUint32() {
    let state = this.state || 0x9e3779b9
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    this.state = state >>> 0
    return this.state
  }

  nextFloat() {
    return this.nextUint32() / 0x1_0000_0000
  }

  integer(min, max) {
    if (!Number.isSafeInteger(min) || !Number.isSafeInteger(max) || max < min) {
      throw new Error("chaos random integer bounds are invalid")
    }
    return min + Math.floor(this.nextFloat() * (max - min + 1))
  }

  shuffle(values) {
    const result = [...values]
    for (let index = result.length - 1; index > 0; index -= 1) {
      const swapIndex = this.integer(0, index)
      const previous = result[index]
      result[index] = result[swapIndex]
      result[swapIndex] = previous
    }
    return result
  }
}

export class DrillChaosTrace {
  constructor(nowMs = () => 0) {
    this.nowMs = nowMs
    this.events = []
  }

  record(kind, details = {}) {
    const event = {
      sequence: this.events.length + 1,
      atMs: this.nowMs(),
      kind,
      details: sanitizeDrillMetadata(details),
    }
    this.events.push(event)
    return event
  }

  snapshot() {
    return this.events.map((event) => ({ ...event, details: structuredClone(event.details) }))
  }

  count(kind) {
    return this.events.filter((event) => event.kind === kind).length
  }
}

export class DrillChaosClock {
  constructor({ trace } = {}) {
    this.currentTimeMs = 0
    this.nextTimerId = 1
    this.tasks = new Map()
    this.trace = trace
  }

  nowMs() {
    return this.currentTimeMs
  }

  setTimeout(callback, delayMs, metadata = {}) {
    if (typeof callback !== "function") throw new Error("chaos clock callback is required")
    if (!Number.isSafeInteger(delayMs) || delayMs < 0) {
      throw new Error("chaos clock delay must be a non-negative integer")
    }
    const id = this.nextTimerId++
    const dueMs = this.currentTimeMs + delayMs
    this.tasks.set(id, { id, dueMs, callback, metadata })
    this.trace?.record("clock.scheduled", { id, dueMs, metadata })
    return id
  }

  clearTimeout(id) {
    const deleted = this.tasks.delete(id)
    if (deleted) this.trace?.record("clock.cancelled", { id })
    return deleted
  }

  pendingCount() {
    return this.tasks.size
  }

  async advanceBy(delayMs, options) {
    if (!Number.isSafeInteger(delayMs) || delayMs < 0) {
      throw new Error("chaos clock advance must be a non-negative integer")
    }
    return this.advanceTo(this.currentTimeMs + delayMs, options)
  }

  async advanceTo(targetMs, { maxSteps = 100_000 } = {}) {
    if (!Number.isSafeInteger(targetMs) || targetMs < this.currentTimeMs) {
      throw new Error("chaos clock target must be monotonic")
    }
    let steps = 0
    while (true) {
      const task = this.nextTask()
      if (!task || task.dueMs > targetMs) break
      if (++steps > maxSteps) throw new Error(`chaos clock exceeded ${maxSteps} steps`)
      await this.runTask(task)
    }
    this.currentTimeMs = targetMs
    return steps
  }

  async runUntilIdle({ maxSteps = 100_000 } = {}) {
    let steps = 0
    while (this.tasks.size > 0) {
      if (++steps > maxSteps) throw new Error(`chaos clock exceeded ${maxSteps} steps with tasks pending`)
      await this.runTask(this.nextTask())
    }
    return steps
  }

  nextTask() {
    return [...this.tasks.values()]
      .sort((left, right) => left.dueMs - right.dueMs || left.id - right.id)[0] ?? null
  }

  async runTask(task) {
    this.tasks.delete(task.id)
    this.currentTimeMs = task.dueMs
    this.trace?.record("clock.fired", { id: task.id, metadata: task.metadata })
    await task.callback()
  }
}

export class DrillChaosProcess {
  constructor({ name, clock, trace }) {
    if (!name) throw new Error("chaos process name is required")
    this.name = name
    this.clock = clock
    this.trace = trace
    this.alive = true
    this.generation = 1
    this.pending = new Set()
    this.staleCallbacksSuppressed = 0
  }

  schedule(label, delayMs, callback) {
    const generation = this.generation
    let timerId = 0
    timerId = this.clock.setTimeout(async () => {
      this.pending.delete(timerId)
      if (!this.alive || generation !== this.generation) {
        this.staleCallbacksSuppressed += 1
        this.trace?.record("process.stale-callback-suppressed", {
          process: this.name,
          label,
          scheduledGeneration: generation,
          currentGeneration: this.generation,
        })
        return
      }
      this.trace?.record("process.callback", { process: this.name, label, generation })
      await callback()
    }, delayMs, { process: this.name, label, generation })
    this.pending.add(timerId)
    return timerId
  }

  kill(reason = "fault injection") {
    if (!this.alive) return
    this.alive = false
    this.generation += 1
    this.trace?.record("process.killed", { process: this.name, reason, generation: this.generation })
  }

  restart() {
    if (this.alive) return
    this.alive = true
    this.generation += 1
    this.trace?.record("process.restarted", { process: this.name, generation: this.generation })
  }

  pendingCount() {
    return this.pending.size
  }
}

export class DrillChaosTransport {
  constructor({ name, clock, trace, deliver, faultPlan = [], maxQueueDepth = 256 }) {
    if (!name) throw new Error("chaos transport name is required")
    if (typeof deliver !== "function") throw new Error("chaos transport deliver callback is required")
    if (!Number.isSafeInteger(maxQueueDepth) || maxQueueDepth < 1) {
      throw new Error("chaos transport maxQueueDepth must be positive")
    }
    validateDrillChaosFaultPlan(faultPlan, `${name} fault plan`)
    this.name = name
    this.clock = clock
    this.trace = trace
    this.deliver = deliver
    this.maxQueueDepth = maxQueueDepth
    this.maxObservedQueueDepth = 0
    this.scheduledDeliveries = 0
    this.partitionedRoutes = new Map()
    this.reorderBuffers = new Map()
    this.faults = faultPlan.map((fault) => ({ ...structuredClone(fault), remaining: fault.times ?? 1 }))
  }

  send(packet) {
    validatePacket(packet, `${this.name} packet`)
    const copy = structuredClone(packet)
    this.trace?.record("transport.sent", { transport: this.name, packet: packetSummary(copy) })
    const partition = this.matchingPartition(copy)
    if (partition) {
      if (partition.mode === "drop") {
        this.trace?.record("transport.partition-drop", { transport: this.name, packet: packetSummary(copy) })
      } else {
        this.ensureQueueCapacity(1)
        partition.packets.push(copy)
        this.observeQueueDepth()
        this.trace?.record("transport.partition-held", { transport: this.name, packet: packetSummary(copy) })
      }
      return
    }
    const fault = this.faults.find((candidate) => candidate.remaining > 0 && matchesPacket(candidate.match, copy))
    if (!fault) {
      this.scheduleDelivery(copy, 0, { faultId: null, copyIndex: 0 })
      return
    }
    const reservedCapacity = fault.kind === "duplicate"
      ? (fault.copies ?? 2)
      : (fault.kind === "delay" || fault.kind === "reorder" ? 1 : 0)
    if (reservedCapacity > 0) this.ensureQueueCapacity(reservedCapacity)
    fault.remaining -= 1
    this.trace?.record("transport.fault-applied", {
      transport: this.name,
      faultId: fault.id,
      faultKind: fault.kind,
      remaining: fault.remaining,
      packet: packetSummary(copy),
    })
    if (fault.kind === "drop") return
    if (fault.kind === "delay") {
      this.scheduleDelivery(copy, fault.delayMs ?? 1, { faultId: fault.id, copyIndex: 0 }, { capacityReserved: true })
      return
    }
    if (fault.kind === "duplicate") {
      const copies = fault.copies ?? 2
      for (let copyIndex = 0; copyIndex < copies; copyIndex += 1) {
        this.scheduleDelivery(
          copy,
          copyIndex * (fault.spacingMs ?? 0),
          { faultId: fault.id, copyIndex },
          { capacityReserved: true },
        )
      }
      return
    }
    if (fault.kind === "reorder") {
      const buffer = this.reorderBuffers.get(fault.id) ?? []
      buffer.push(copy)
      this.reorderBuffers.set(fault.id, buffer)
      this.observeQueueDepth()
      if (buffer.length >= (fault.window ?? 2)) this.releaseReorderBuffer(fault.id, fault.spacingMs ?? 0)
      return
    }
    throw new Error(`fault ${fault.id} cannot be applied by a transport`)
  }

  partition({ from = "*", to = "*", mode = "hold" } = {}) {
    if (mode !== "hold" && mode !== "drop") throw new Error("partition mode must be hold or drop")
    const key = routeKey(from, to)
    if (!this.partitionedRoutes.has(key)) {
      this.partitionedRoutes.set(key, { from, to, mode, packets: [] })
      this.trace?.record("transport.partitioned", { transport: this.name, from, to, mode })
    }
  }

  reconnect({ from = "*", to = "*", release = "deliver" } = {}) {
    if (release !== "deliver" && release !== "drop") throw new Error("reconnect release must be deliver or drop")
    const partition = this.partitionedRoutes.get(routeKey(from, to))
    if (!partition) return
    this.partitionedRoutes.delete(routeKey(from, to))
    this.trace?.record("transport.reconnected", {
      transport: this.name,
      from,
      to,
      release,
      heldPackets: partition.packets.length,
    })
    if (release === "deliver") {
      partition.packets.forEach((packet) => this.scheduleDelivery(packet, 0, { faultId: "route-partition", copyIndex: 0 }))
    } else {
      partition.packets.forEach((packet) => this.trace?.record("transport.partition-release-drop", {
        transport: this.name,
        packet: packetSummary(packet),
      }))
    }
  }

  flushReorderBuffers() {
    for (const faultId of [...this.reorderBuffers.keys()]) this.releaseReorderBuffer(faultId, 0)
  }

  pendingCount() {
    return this.scheduledDeliveries
      + [...this.partitionedRoutes.values()].reduce((sum, partition) => sum + partition.packets.length, 0)
      + [...this.reorderBuffers.values()].reduce((sum, packets) => sum + packets.length, 0)
  }

  scheduleDelivery(packet, delayMs, metadata, { capacityReserved = false } = {}) {
    if (!capacityReserved) this.ensureQueueCapacity(1)
    this.scheduledDeliveries += 1
    this.observeQueueDepth()
    this.clock.setTimeout(async () => {
      this.scheduledDeliveries -= 1
      this.trace?.record("transport.delivered", {
        transport: this.name,
        packet: packetSummary(packet),
        ...metadata,
      })
      await this.deliver(structuredClone(packet), metadata)
    }, delayMs, { transport: this.name, messageId: packet.messageId, ...metadata })
  }

  releaseReorderBuffer(faultId, spacingMs) {
    const packets = this.reorderBuffers.get(faultId) ?? []
    this.reorderBuffers.delete(faultId)
    packets.reverse().forEach((packet, index) => {
      this.scheduleDelivery(packet, index * spacingMs, { faultId, copyIndex: 0 })
    })
  }

  matchingPartition(packet) {
    return [...this.partitionedRoutes.values()].find((partition) => (
      routePartMatches(partition.from, packet.from) && routePartMatches(partition.to, packet.to)
    )) ?? null
  }

  ensureQueueCapacity(additional) {
    if (this.pendingCount() + additional > this.maxQueueDepth) {
      throw new Error(`${this.name} chaos transport queue exceeded ${this.maxQueueDepth}`)
    }
  }

  observeQueueDepth() {
    this.maxObservedQueueDepth = Math.max(this.maxObservedQueueDepth, this.pendingCount())
  }
}

export function createDrillChaosReplayBundle({
  scenario,
  random,
  faultPlan,
  trace,
  invariants,
  metadata = {},
}) {
  const events = trace.snapshot()
  const bundle = {
    schema: DRILL_CHAOS_REPLAY_SCHEMA,
    scenario,
    seed: random.seed,
    seedState: random.state,
    faultPlan: structuredClone(faultPlan),
    invariants,
    trace: events,
    summary: {
      traceEvents: events.length,
      faultsApplied: events.filter((event) => (
        event.kind === "transport.fault-applied" || event.kind === "chaos.fault-applied"
      )).length,
      staleCallbacksSuppressed: events.filter((event) => event.kind === "process.stale-callback-suppressed").length,
    },
    metadata: sanitizeDrillMetadata(metadata),
  }
  return validateDrillChaosReplayBundle(bundle)
}

function hashSeed(seed) {
  let hash = 0x811c9dc5
  for (const byte of new TextEncoder().encode(seed)) {
    hash ^= byte
    hash = Math.imul(hash, 0x01000193)
  }
  return hash >>> 0
}

function validatePacket(packet, source) {
  if (!packet || typeof packet !== "object" || Array.isArray(packet)) throw new Error(`${source} is invalid`)
  for (const key of ["messageId", "from", "to", "type"]) {
    if (typeof packet[key] !== "string" || packet[key].trim().length === 0) {
      throw new Error(`${source}.${key} is invalid`)
    }
  }
  if (packet.operationId !== undefined && (typeof packet.operationId !== "string" || packet.operationId.trim().length === 0)) {
    throw new Error(`${source}.operationId is invalid`)
  }
}

function packetSummary(packet) {
  return {
    messageId: packet.messageId,
    operationId: packet.operationId ?? null,
    from: packet.from,
    to: packet.to,
    type: packet.type,
  }
}

function matchesPacket(match = {}, packet) {
  return Object.entries(match).every(([key, value]) => key === "channel"
    ? value === packet.channel
    : value === packet[key])
}

function routeKey(from, to) {
  return `${from}\u0000${to}`
}

function routePartMatches(expected, actual) {
  return expected === "*" || expected === actual
}
