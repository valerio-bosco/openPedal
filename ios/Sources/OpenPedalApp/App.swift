import SwiftUI

@main
struct OpenPedalApp: App {
    @StateObject private var host = AudioHost()
    @State private var store: Store?

    var body: some Scene {
        WindowGroup {
            Group {
                switch host.state {
                case .starting:
                    ProgressView("Starting audio engine...")
                case .running(let port):
                    Group {
                        if let store { RootView(store: store) } else { ProgressView() }
                    }
                    .onAppear { if store == nil { store = Store(port: port) } }
                case .failed(let message):
                    VStack(spacing: 12) {
                        Text("openPedal could not start").font(.headline)
                        Text(message).font(.callout).multilineTextAlignment(.center)
                        Button("Try again") { host.start() }.buttonStyle(.glass)
                    }
                    .padding()
                }
            }
            .preferredColorScheme(.dark)
            .task { host.start() }
        }
    }
}

struct RootView: View {
    let store: Store
    @State private var showAdd = false
    @State private var showAbout = false

    private var tint: Color {
        guard let n = store.selected else { return Color(hex: 0xff8a3d) }
        return enamel(for: n.type, category: store.item(for: n)?.category)
    }

    var body: some View {
        ZStack {
            Ambient(tint: tint)
            BoardView(store: store, showAdd: $showAdd)
            VStack(spacing: 0) {
                ToolbarView(store: store, showAbout: $showAbout)
                    .padding(.horizontal, 20).padding(.top, 8)
                if let text = store.banner {
                    Label(text, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout.weight(.semibold))
                        .padding(.horizontal, 18).padding(.vertical, 10)
                        .glassEffect(.regular.tint(.red.opacity(0.35)), in: .capsule)
                        .padding(.top, 10)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
                Spacer()
                InspectorView(store: store)
                    .padding(.horizontal, 20).padding(.bottom, 8)
            }
            .animation(.smooth, value: store.banner)

            if store.protocolTooNew {
                Text("This app is older than the openPedal core (protocol v\(store.serverProtocol ?? 0)). Update the app.")
                    .multilineTextAlignment(.center).padding(28)
                    .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 28))
            } else if !store.connected {
                ProgressView("Connecting...").padding(24).glassEffect(.regular, in: RoundedRectangle(cornerRadius: 28))
            }
        }
        .sheet(isPresented: $showAdd) { AddPedalSheet(store: store) }
        .sheet(isPresented: $showAbout) { AboutSheet(store: store) }
    }
}

/// Dark backdrop with a glow in the colour of the selected pedal (glass needs colour behind it).
struct Ambient: View {
    let tint: Color
    var body: some View {
        ZStack {
            LinearGradient(colors: [Color(hex: 0x121420), Color(hex: 0x0a0b10), Color(hex: 0x14100c)], startPoint: .top, endPoint: .bottom)
            RadialGradient(colors: [tint.opacity(0.5), .clear], center: UnitPoint(x: 0.3, y: 0.7), startRadius: 0, endRadius: 600)
            RadialGradient(colors: [tint.opacity(0.25), .clear], center: UnitPoint(x: 0.85, y: 0.15), startRadius: 0, endRadius: 500)
        }
        .ignoresSafeArea()
        .animation(.easeInOut(duration: 0.6), value: tint)
    }
}
