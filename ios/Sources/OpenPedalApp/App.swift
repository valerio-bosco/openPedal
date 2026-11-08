import SwiftUI

@main
struct OpenPedalApp: App {
    @StateObject private var host = AudioHost()

    var body: some Scene {
        WindowGroup {
            Group {
                switch host.state {
                case .starting:
                    ProgressView("Starting audio engine...")
                case .running(let port):
                    // The UI is served by the Rust core on loopback.
                    WebView(url: URL(string: "http://127.0.0.1:\(port)")!)
                        .ignoresSafeArea()
                case .failed(let message):
                    VStack(spacing: 12) {
                        Text("openPedal could not start").font(.headline)
                        Text(message).font(.callout).multilineTextAlignment(.center)
                        Button("Try again") { host.start() }
                    }
                    .padding()
                }
            }
            .preferredColorScheme(.dark)
            .task { host.start() }
        }
    }
}
