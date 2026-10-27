import SwiftUI

public struct ContentView: View {
    @State private var model = CharioxAppModel()

    public init() {}

    public var body: some View {
        CharioxRootView(model: model)
    }
}
