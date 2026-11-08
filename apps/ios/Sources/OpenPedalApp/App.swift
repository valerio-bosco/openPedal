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
    @State private var expanded = false
    @State private var liveHeight: CGFloat?

    private var tint: Color {
        // Structure only: turning a knob must not redraw the whole screen.
        guard let type = store.selectedType else { return Color(hex: 0xff8a3d) }
        return enamel(for: type, category: store.item(kind: type)?.category.rawValue)
    }

    /// Toolbar row (8 top + 52) and the inspector's outer padding (8 + 8): fixed chrome around the board.
    private static let toolbarBand: CGFloat = 60
    private static let inspectorPadding: CGFloat = 16

    var body: some View {
        GeometryReader { geo in
            let showInspector = store.hasPedals
            let ih = showInspector ? (liveHeight ?? (expanded ? InspectorDetent.expanded : InspectorDetent.compact)) : 0
            // The pedals scale to the detent, not to every point of a grabber drag (which would re-lay out
            // the whole board on each touch event).
            let detentHeight = showInspector ? (expanded ? InspectorDetent.expanded : InspectorDetent.compact) : 0
            let boardHeight = max(0, geo.size.height - Self.toolbarBand - (showInspector ? detentHeight + Self.inspectorPadding : 0))
            let unit = pedalUnit(forBoardHeight: boardHeight)
            ZStack(alignment: .top) {
                Ambient(tint: tint)
                VStack(spacing: 0) {
                    ToolbarView(store: store, showAbout: $showAbout)
                        .frame(height: 52)
                        .padding(.horizontal, 20).padding(.top, 8)
                    // The board is laid out once at its final size; the resize is a transform (BoardScale)
                    // animated with the inspector spring, so no pedal re-renders meanwhile.
                    BoardView(store: store, showAdd: $showAdd, pf: unit)
                        .frame(maxWidth: .infinity)
                        .frame(height: boardHeight)
                        .transaction(value: boardHeight) { $0.animation = nil }
                        .modifier(BoardScale(u: unit, h: boardHeight, targetU: unit, targetH: boardHeight))
                    Spacer(minLength: 0)
                }
                .overlay(alignment: .top) {
                    if let text = store.banner {
                        Label(text, systemImage: "exclamationmark.triangle.fill")
                            .font(.callout.weight(.semibold))
                            .padding(.horizontal, 18).padding(.vertical, 10)
                            .glassEffect(.regular.tint(.red.opacity(0.35)), in: .capsule)
                            .padding(.top, 72)
                            .transition(.move(edge: .top).combined(with: .opacity))
                    }
                }
                .animation(.smooth, value: store.banner)

                // Floats over the bottom of the screen: its height animates without moving the board.
                if showInspector {
                    InspectorView(store: store, expanded: $expanded, liveHeight: $liveHeight, height: ih, detentHeight: detentHeight)
                        .padding(.horizontal, 16).padding(.vertical, 8)
                        .frame(maxHeight: .infinity, alignment: .bottom)
                }

                if store.protocolTooNew {
                    Text("This app is older than the openPedal core (protocol v\(store.serverProtocol ?? 0)). Update the app.")
                        .multilineTextAlignment(.center).padding(28)
                        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 28))
                } else if !store.connected {
                    ProgressView("Connecting...").padding(24).glassEffect(.regular, in: RoundedRectangle(cornerRadius: 28))
                }
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
