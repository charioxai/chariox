import Foundation

enum Refusal: Error { case disabled, target, secure, text, arguments, permission, native, ownedInput }

struct RefusalDetail: Error {
    let kind: Refusal
    let reason: String
}
func tagged(_ kind: Refusal, _ reason: String) -> RefusalDetail {
    RefusalDetail(kind: kind, reason: reason)
}
func refusalKind(_ error: Error) -> Refusal {
    (error as? RefusalDetail)?.kind ?? (error as? Refusal) ?? .native
}
func refusalMessage(_ error: Error) -> String {
    let kind = refusalKind(error)
    let reason = (error as? RefusalDetail)?.reason ?? "unexpected-error"
    return "refused: \(kind) (\(reason))" +
        (kind == .ownedInput ? "; unresolved owned input; owner reset required" : "")
}
enum Operation: Equatable { case capture(String), read, click, scroll, text(String) }
struct Request {
    var enabled = false
    var ownerWindow = false
    var pid: Int32 = 0
    var window: UInt32 = 0
    var target = "ordinary"
    var clickPoint: CGPoint?
    var clickAtPointer = false
    var operations: [Operation] = []
    var publicTextDrillDocument: String?
    var closePublicTextDrill = false
    var publicDrillLegacyPointer: CGPoint?
    var publicDrillAppKitPointer: CGPoint?
    static func parse(_ arguments: [String]) throws -> Request {
        var request = Request(), index = 0
        var ownerPID = false, fixturePID = false
        func value() throws -> String {
            index += 1
            guard index < arguments.count else { throw tagged(.arguments, "missing-argument") }
            return arguments[index]
        }
        while index < arguments.count {
            switch arguments[index] {
            case "--enable-fixture", "--enable-owner-window":
                guard !request.enabled else { throw tagged(.arguments, "duplicate-enable") }
                request.enabled = true
                request.ownerWindow = arguments[index] == "--enable-owner-window"
                if request.ownerWindow { request.target = "focused" }
            case "--fixture-pid": fixturePID = true; request.pid = Int32(try value()) ?? 0
            case "--owner-pid": ownerPID = true; request.pid = Int32(try value()) ?? 0
            case "--window-id": request.window = UInt32(try value()) ?? 0
            case "--target": request.target = try value()
            case "--capture": request.operations.append(.capture(try value()))
            case "--ax-read": request.operations.append(.read)
            case "--click": request.operations.append(.click)
            case "--click-at-pointer":
                guard !request.clickAtPointer, request.clickPoint == nil else { throw tagged(.arguments, "duplicate-pointer-point") }
                request.clickAtPointer = true
            case "--click-at":
                let x = Double(try value()), y = Double(try value())
                guard let x, let y, x.isFinite, y.isFinite, request.clickPoint == nil, !request.clickAtPointer else { throw tagged(.arguments, "invalid-click-point") }
                request.clickPoint = CGPoint(x: x, y: y)
            case "--scroll": request.operations.append(.scroll)
            case "--text": request.operations.append(.text(try value()))
            case "--public-text-drill-document":
                guard request.publicTextDrillDocument == nil else { throw tagged(.arguments, "duplicate-public-document") }
                request.publicTextDrillDocument = try value()
            case "--close-public-text-drill": request.closePublicTextDrill = true
            default: throw tagged(.arguments, "unknown-argument")
            }
            index += 1
        }
        guard !(ownerPID && fixturePID), !ownerPID || request.ownerWindow,
              !fixturePID || !request.ownerWindow else { throw tagged(.arguments, "mixed-scope") }
        guard (request.clickPoint == nil && !request.clickAtPointer) || request.operations == [.click] else { throw tagged(.arguments, "point-needs-single-click") }
        if request.publicTextDrillDocument != nil || request.closePublicTextDrill {
            guard request.publicTextDrillDocument != nil, request.ownerWindow,
                  request.target == "focused", request.operations == [.read] else {
                throw tagged(.arguments, "public-drill-needs-owner-read")
            }
        }
        return request
    }
}
protocol NativeSource {
    func validateTarget(_ request: Request) throws
    func checkPermission(_ operation: Operation) throws
    func perform(_ operation: Operation, request: Request) async throws -> String
}
func checkOperationPermissions(_ operation: Operation, screenCaptureAccess: () -> Bool,
                               accessibilityAccess: () -> Bool) throws {
    if case .capture = operation {
        guard screenCaptureAccess() else { throw tagged(.permission, "screen-recording-denied") }
    }
    guard accessibilityAccess() else { throw tagged(.permission, "accessibility-denied") }
}
func run(_ request: Request, source: NativeSource) async throws -> [String] {
    guard request.enabled else { throw tagged(.disabled, "scope-disabled") }
    guard request.pid > 0, request.window > 0,
          (request.ownerWindow ? ["focused"] : ["ordinary", "secure", "button", "scroll", "text"]).contains(request.target) else { throw tagged(.target, "invalid-scope") }
    guard request.target != "secure" else { throw tagged(.secure, "secure-target") }
    for operation in request.operations {
        if case .text(let text) = operation { _ = try textUnits(text) }
    }
    var result: [String] = []
    for operation in request.operations {
        try source.validateTarget(request)
        try source.checkPermission(operation)
        result.append(try await source.perform(operation, request: request))
    }
    return result
}
func textUnits(_ text: String) throws -> [UInt16] {
    // Refuse the whole string instead of truncating a grapheme or surrogate pair.
    let units = Array(text.utf16)
    guard !units.isEmpty, units.count <= 20,
          !text.unicodeScalars.contains(where: {
              CharacterSet.newlines.contains($0) ||
              (CharacterSet.controlCharacters.contains($0) && $0.value != 0x200C && $0.value != 0x200D)
          })
    else { throw tagged(.text, "invalid-text") }
    return units
}

func checkWindowBinding(pid: Int32, frontmostPID: Int32?,
                        axMatches: Int, cgMatches: Int, focused: Bool) throws {
    guard pid > 0 else { throw tagged(.target, "invalid-pid") }
    guard frontmostPID == pid else { throw tagged(.target, "not-frontmost") }
    guard axMatches == 1 else { throw tagged(.target, "ax-window-ambiguous") }
    guard cgMatches == 1 else { throw tagged(.target, "cg-window-ambiguous") }
    guard focused else { throw tagged(.target, "window-not-focused") }
}
