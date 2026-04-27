import SwiftUI

public struct ContentView: View {
    @State private var model = ArrobaAppModel()

    public init() {}

    public var body: some View {
        ArrobaRootView(model: model)
    }
}
