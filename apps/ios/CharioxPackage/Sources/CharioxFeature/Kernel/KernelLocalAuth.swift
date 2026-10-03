import Foundation

enum KernelLocalAuth {
    static func connectionRequest(to endpoint: URL) -> URLRequest {
        #if targetEnvironment(simulator)
        return connectionRequest(to: endpoint, environment: ProcessInfo.processInfo.environment)
        #else
        return URLRequest(url: endpoint)
        #endif
    }

    static func connectionRequest(to endpoint: URL, environment: [String: String]) -> URLRequest {
        var request = URLRequest(url: endpoint)
        guard ["ws", "wss"].contains(endpoint.scheme?.lowercased() ?? ""),
              ["localhost", "127.0.0.1", "[::1]", "::1"].contains(endpoint.host?.lowercased() ?? ""),
              endpoint.user == nil, endpoint.password == nil
        else { return request }

        // simctl strips SIMCTL_CHILD_ when passing variables to the app.
        let file = environment["CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE"]
            ?? environment["SIMCTL_CHILD_CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE"]
        let fallback = environment["CHARIOX_KERNEL_LOCAL_AUTH_TOKEN"]
            ?? environment["SIMCTL_CHILD_CHARIOX_KERNEL_LOCAL_AUTH_TOKEN"]
        let fileToken = file.flatMap { try? String(contentsOfFile: $0, encoding: .utf8) }
        if let token = validToken(fileToken) ?? validToken(fallback) {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private static func validToken(_ value: String?) -> String? {
        guard let token = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              !token.isEmpty, token.utf8.count <= 8 * 1024,
              token.utf8.allSatisfy({ $0 >= 0x21 && $0 <= 0x7e })
        else { return nil }
        return token
    }
}
