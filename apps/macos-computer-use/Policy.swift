import Foundation

enum Refusal: Error { case disabled, target, secure, text, arguments, permission, native }
enum Operation: Equatable { case capture(String), read, click, scroll, text(String) }
struct Request {
    var enabled = false
    var pid: Int32 = 0
    var window: UInt32 = 0
    var target = "ordinary"
    var operations: [Operation] = []
    static func parse(_ arguments: [String]) throws -> Request {
        var request = Request(), index = 0
        func value() throws -> String {
            index += 1
            guard index < arguments.count else { throw Refusal.arguments }
            return arguments[index]
        }
        while index < arguments.count {
            switch arguments[index] {
            case "--enable-fixture": request.enabled = true
            case "--fixture-pid": request.pid = Int32(try value()) ?? 0
            case "--window-id": request.window = UInt32(try value()) ?? 0
            case "--target": request.target = try value()
            case "--capture": request.operations.append(.capture(try value()))
            case "--ax-read": request.operations.append(.read)
            case "--click": request.operations.append(.click)
            case "--scroll": request.operations.append(.scroll)
            case "--text": request.operations.append(.text(try value()))
            default: throw Refusal.arguments
            }
            index += 1
        }
        return request
    }
}
protocol NativeSource {
    func validateFixture(_ request: Request) throws
    func perform(_ operation: Operation, request: Request) async throws -> String
}
func run(_ request: Request, source: NativeSource) async throws -> [String] {
    guard request.enabled else { throw Refusal.disabled }
    guard request.pid > 0, request.window > 0,
          ["ordinary", "secure", "button", "scroll"].contains(request.target) else { throw Refusal.target }
    guard request.target != "secure" else { throw Refusal.secure }
    for operation in request.operations {
        if case .text(let text) = operation { _ = try textUnits(text) }
    }
    var result: [String] = []
    for operation in request.operations {
        try source.validateFixture(request)
        result.append(try await source.perform(operation, request: request))
    }
    return result
}
func textUnits(_ text: String) throws -> [UInt16] {
    let units = Array(text.utf16)
    guard !units.isEmpty, units.count <= 64,
          !text.unicodeScalars.contains(where: {
              CharacterSet.newlines.contains($0) ||
              (CharacterSet.controlCharacters.contains($0) && $0.value != 0x200C && $0.value != 0x200D)
          })
    else { throw Refusal.text }
    return units
}
