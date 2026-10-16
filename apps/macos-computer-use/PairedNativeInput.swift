import AppKit

@MainActor final class PairedNativeInput {
    let observer: any PhysicalInputObserver
    let seat: NativeInputSeat
    private var started = false
    private let foreignHeld: () -> Bool
    private let execute: (NativeInputPlan, Request, MacSource, any PhysicalInputObserver, NativeInputSeat) async throws -> [String]
    init(observer: (any PhysicalInputObserver)? = nil,
         foreignHeld: @escaping () -> Bool = foreignInputHeld, journal: OwnedInputJournal? = nil,
         execute: @escaping (NativeInputPlan, Request, MacSource, any PhysicalInputObserver, NativeInputSeat) async throws -> [String] = {
             try await $2.performPhysical($0, request: $1, observer: $3, seat: $4)
         }) {
        self.observer = observer ?? PhysicalHumanObserver(fence: PhysicalHumanFence()); self.foreignHeld = foreignHeld; self.execute = execute
        seat = NativeInputSeat(journal: journal)
    }
    var paused: Bool { observer.fence.paused || !observer.fence.observerReady }
    func start() throws {
        guard !started else { return }
        try observer.start()
        try seat.recoverStaleRelease { event in observer.fence.tag(event); event.post(tap: .cgSessionEventTap) }
        started = true
    }
    func stop() { observer.stop() }
    func reset(explicitRelease: Bool) throws {
        guard !seat.unresolved else { throw tagged(.ownedInput, "owned-release-failed") }
        if explicitRelease { try observer.fence.release(foreignHeld: foreignHeld()) }
    }
    func perform(_ input: [String: Any], scope: Request, surface: NativeSurface, source: MacSource) async throws -> [String] {
        try observer.check()
        let plan = try nativeInputPlan(input, surface: surface)
        return try await execute(plan, scope, source, observer, seat)
    }
}
