import ConsoleCore
import SwiftUI

@main
struct ConsoleApp: App {
    @StateObject private var model = AppModel(vault: KeychainVault())

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
        }
    }
}
