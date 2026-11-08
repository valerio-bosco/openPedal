import SwiftUI

/// Pedal size unit range: one "em" of the pedal layout, in points. Pedals are 9.4 x 15.75 em.
let pedalUnitRange: ClosedRange<CGFloat> = 9...16

/// Unit that makes the pedals fit a board of the given height (room for lift and selection ring included).
func pedalUnit(forBoardHeight h: CGFloat) -> CGFloat {
    min(max((h - 40) / 17, pedalUnitRange.lowerBound), pedalUnitRange.upperBound)
}

private struct DragInfo: Equatable {
    var id: String
    var dx: CGFloat
}

/// Horizontal pedalboard between the toolbar and the inspector: input jack, pedals in signal order,
/// output jack, "+". Pedals are vertically centred, scale with `pf` and can be reordered by
/// long-press and drag. Animatable so `pf` changes smoothly (real layout, not a blurry transform).
struct BoardView: View, Animatable {
    let store: Store
    @Binding var showAdd: Bool
    /// Changes when the inspector snaps to another detent: the selected pedal is scrolled back into view.
    var expanded: Bool
    var pf: CGFloat
    var animatableData: CGFloat {
        get { pf }
        set { pf = newValue }
    }

    @GestureState(resetTransaction: Transaction(animation: .spring(response: 0.35, dampingFraction: 0.8)))
    private var drag: DragInfo?

    private var spacing: CGFloat { pf * 1.25 }
    private var step: CGFloat { pf * 9.4 + spacing }

    var body: some View {
        GeometryReader { geo in
            ZStack {
                if store.gotState && !store.hasPedals {
                    EmptyChainCard(store: store, showAdd: $showAdd)
                } else {
                    lane(width: geo.size.width, height: geo.size.height)
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
            .overlay(alignment: .top) {
                Text("Drop to move the pedal")
                    .font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                    .padding(.horizontal, 16).padding(.vertical, 9)
                    .glassEffect(.regular, in: .capsule)
                    .opacity(drag == nil ? 0 : 1)
                    .animation(.easeOut(duration: 0.18), value: drag == nil)
                    .allowsHitTesting(false)
            }
        }
    }

    private func lane(width: CGFloat, height: CGFloat) -> some View {
        let nodes = store.chain.nodes
        let pedalIDs = nodes.filter { $0.type != "input" && $0.type != "output" }.map(\.id)
        let target = dragTarget(pedalIDs)
        return ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: spacing) {
                    ForEach(nodes) { node in
                        Group {
                            if node.type == "input" || node.type == "output" {
                                JackView(store: store, node: node)
                            } else {
                                pedal(node, pedalIDs: pedalIDs, target: target)
                            }
                        }
                        .id(node.id)
                    }
                    if store.canEditChain {
                        Button { showAdd = true } label: {
                            Image(systemName: "plus").font(.title.weight(.semibold)).frame(width: 64, height: 64)
                        }
                        .buttonStyle(.glass)
                        .buttonBorderShape(.circle)
                        .accessibilityLabel("Add pedal")
                    }
                }
                .coordinateSpace(.named("lane"))
                .frame(minWidth: max(0, width - 80), minHeight: height)
                .background(alignment: .center) { Wire() }
            }
            .contentMargins(.horizontal, 40, for: .scrollContent)
            .scrollClipDisabled()
            .scrollDisabled(drag != nil)
            .sensoryFeedback(.selection, trigger: target)
            .onChange(of: store.selectedID) { _, id in
                if let id { withAnimation(.smooth) { proxy.scrollTo(id, anchor: .center) } }
            }
            .task(id: expanded) {
                // Let the inspector finish its spring before centring the selection again.
                try? await Task.sleep(for: .milliseconds(380))
                if let id = store.selectedID { withAnimation(.smooth) { proxy.scrollTo(id, anchor: .center) } }
            }
        }
    }

    // MARK: drag and drop

    /// Slot the dragged pedal would land on, nil when nothing is being dragged.
    private func dragTarget(_ ids: [String]) -> Int? {
        guard let d = drag, let from = ids.firstIndex(of: d.id) else { return nil }
        return slot(from: from, dx: d.dx, count: ids.count)
    }

    private func slot(from: Int, dx: CGFloat, count: Int) -> Int {
        min(max(from + Int((dx / step).rounded()), 0), max(count - 1, 0))
    }

    /// Horizontal offset of a pedal: follows the finger when dragged, slides aside to make room otherwise.
    private func offsetX(_ id: String, _ ids: [String], _ target: Int?) -> CGFloat {
        guard let d = drag, let target, let from = ids.firstIndex(of: d.id), let k = ids.firstIndex(of: id) else { return 0 }
        if id == d.id { return d.dx }
        if from < target, k > from, k <= target { return -step }
        if from > target, k >= target, k < from { return step }
        return 0
    }

    private func reorder(_ id: String) -> some Gesture {
        LongPressGesture(minimumDuration: 0.3)
            .sequenced(before: DragGesture(minimumDistance: 0, coordinateSpace: .named("lane")))
            .updating($drag) { value, state, _ in
                if case .second(true, let d) = value { state = DragInfo(id: id, dx: d?.translation.width ?? 0) }
            }
            .onEnded { value in
                guard case .second(true, let d?) = value else { return }
                let ids = store.chain.nodes.filter { $0.type != "input" && $0.type != "output" }.map(\.id)
                guard let from = ids.firstIndex(of: id) else { return }
                let to = slot(from: from, dx: d.translation.width, count: ids.count)
                withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) { store.moveTo(id, slot: to) }
            }
    }

    private func pedal(_ node: NodeSpec, pedalIDs: [String], target: Int?) -> some View {
        let lifted = drag?.id == node.id
        let dx = offsetX(node.id, pedalIDs, target)
        return PedalView(store: store, node: node, u: pf, lifted: lifted)
            .offset(x: dx)
            .animation(lifted ? nil : .spring(response: 0.3, dampingFraction: 0.85), value: dx)
            .zIndex(lifted ? 1 : 0)
            .simultaneousGesture(reorder(node.id), including: store.canEditChain ? .all : .subviews)
    }
}

/// The patch cable running behind the pedals.
private struct Wire: View {
    var body: some View {
        Capsule()
            .fill(LinearGradient(colors: [Color(white: 0.1), Color(white: 0.22), Color(white: 0.1)], startPoint: .leading, endPoint: .trailing))
            .frame(height: 3)
            .padding(.horizontal, 30)
            .shadow(color: .white.opacity(0.06), radius: 0, y: 1)
    }
}

private struct EmptyChainCard: View {
    let store: Store
    @Binding var showAdd: Bool
    var body: some View {
        VStack(spacing: 14) {
            Text("This preset has no pedals")
                .font(.system(size: 22, weight: .bold, design: .rounded))
            Text("Your guitar goes straight to the amp. Add a pedal to start building the sound.")
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 340)
            Button { showAdd = true } label: {
                Label("Add pedal", systemImage: "plus").font(.body.weight(.semibold))
            }
            .buttonStyle(.glassProminent)
            .controlSize(.large)
            .disabled(!store.canEditChain)
            HStack(spacing: 14) {
                Label("Guitar", systemImage: "guitars")
                Capsule().fill(.white.opacity(0.2)).frame(width: 60, height: 2)
                Label("Amp", systemImage: "hifispeaker.fill")
            }
            .font(.footnote).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 36).padding(.vertical, 28)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }
}

/// Input ("Guitar") and output ("Amp") ends of the chain.
struct JackView: View {
    let store: Store
    let node: NodeSpec
    var body: some View {
        let isSel = store.selectedID == node.id
        let label = node.type == "input" ? "Guitar" : "Amp"
        Image(systemName: node.type == "input" ? "guitars" : "hifispeaker.fill")
            .font(.system(size: 26))
            .foregroundStyle(.white.opacity(isSel ? 1 : 0.75))
            .frame(width: 64, height: 64)
            .glassEffect(isSel ? .regular.tint(.white.opacity(0.18)) : .regular, in: .circle)
            // The caption hangs below so the jack's centre stays on the cable.
            .overlay(alignment: .bottom) {
                Text(label).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .fixedSize().offset(y: 22)
            }
            .contentShape(Circle())
            .onTapGesture { store.selectedID = node.id }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(label)
            .accessibilityAddTraits(.isButton)
    }
}

/// Short mini-knob caption: known long labels are mapped, otherwise the first word (max 7 characters).
func shortLabel(_ label: String) -> String {
    let known: [String: String] = [
        "max ir length": "IR len", "resonance q": "Res Q", "resonance": "Reson", "high cut": "Hi cut",
        "threshold": "Thresh", "frequency": "Freq", "feedback": "Fdbk", "pre-delay": "Pre-dly",
        "echo tone": "Tone", "brightness": "Bright", "tracking": "Track", "damping": "Damp",
        "presence": "Pres", "presence freq": "Pres f", "mid freq": "Mid f",
    ]
    if let s = known[label.lowercased()] { return s }
    let first = label.split(whereSeparator: { $0 == " " || $0 == "-" }).first.map(String.init) ?? label
    return String(first.prefix(7))
}

/// A pedal. Every dimension is a multiple of `u` (1 em), so all pedals share one layout and the board
/// can scale them without a transform. Selection never changes the size: it lifts the pedal and draws
/// a ring outside the body.
struct PedalView: View {
    let store: Store
    let node: NodeSpec
    let u: CGFloat
    var lifted = false

    var body: some View {
        let item = store.item(for: node)
        let color = enamel(for: node.type, category: item?.category)
        let isSel = store.selectedID == node.id
        let numbers = Array((item?.params ?? []).filter { $0.kind == .number }.prefix(4))
        let on = !node.bypass
        let shape = RoundedRectangle(cornerRadius: 1.3 * u, style: .continuous)

        ZStack(alignment: .top) {
            Text((item?.label ?? node.type).uppercased())
                .font(.system(size: 0.82 * u, weight: .heavy, design: .rounded))
                .tracking(0.1 * u)
                .lineLimit(1).minimumScaleFactor(0.6)
                .foregroundStyle(.black.opacity(0.62))
                .frame(width: 8.2 * u, height: 1.1 * u)
                .padding(.top, 0.85 * u)

            knobZone(numbers)
                .frame(width: 8.4 * u, height: 6.4 * u)
                .padding(.top, 2.6 * u)

            Circle()
                .fill(on ? Color(hex: 0xff3b30) : Color(hex: 0x4a0d0d))
                .frame(width: 0.62 * u, height: 0.62 * u)
                .shadow(color: on ? Color(hex: 0xff3b30) : .clear, radius: 0.55 * u)
                .overlay(Circle().stroke(.black.opacity(0.35), lineWidth: 0.5))
                .padding(.top, 9.55 * u)

            Button { store.setBypass(node.id, on) } label: { Footswitch(u: u) }
                .buttonStyle(.plain)
                .frame(maxHeight: .infinity, alignment: .bottom)
                .padding(.bottom, 1.25 * u - 8)
                .accessibilityLabel(on ? "Bypass \(item?.label ?? node.type)" : "Enable \(item?.label ?? node.type)")
        }
        .frame(width: 9.4 * u, height: 15.75 * u)
        .background {
            shape
                .fill(LinearGradient(colors: [color.opacity(0.95), color.mix(with: .black, by: 0.35)], startPoint: .top, endPoint: .bottom))
                .overlay(shape.stroke(LinearGradient(colors: [.white.opacity(0.55), .black.opacity(0.3)], startPoint: .top, endPoint: .bottom), lineWidth: 1.2))
                .shadow(color: lifted ? .black.opacity(0.6) : (isSel ? color.opacity(0.6) : .black.opacity(0.5)),
                        radius: lifted ? 1.8 * u : (isSel ? 1.6 * u : 0.8 * u), y: lifted ? 1.4 * u : 0.6 * u)
                .saturation(on ? 1 : 0.35)
                .brightness(on ? 0 : -0.12)
        }
        // Selection ring: drawn outside the body, never part of the layout.
        .background {
            RoundedRectangle(cornerRadius: 1.65 * u, style: .continuous)
                .stroke(.white.opacity(0.9), lineWidth: 2.5)
                .shadow(color: color.opacity(0.7), radius: 1.2 * u)
                .padding(-0.42 * u)
                .opacity(isSel && !lifted ? 1 : 0)
        }
        .scaleEffect(lifted ? 1.06 : 1)
        .offset(y: lifted ? -0.9 * u : (isSel ? -0.45 * u : 0))
        .animation(.spring(response: 0.35, dampingFraction: 0.7), value: isSel)
        .animation(.spring(response: 0.3, dampingFraction: 0.75), value: lifted)
        .animation(.easeOut(duration: 0.2), value: on)
        .contentShape(shape)
        .onTapGesture { store.selectedID = node.id }
        .contextMenu {
            if store.canEditChain {
                Button("Move left", systemImage: "arrow.left") { store.movePedal(node.id, by: -1) }
                Button("Move right", systemImage: "arrow.right") { store.movePedal(node.id, by: 1) }
                Button("Remove", systemImage: "trash", role: .destructive) { store.removePedal(node.id) }
            }
        }
        .sensoryFeedback(.impact(weight: .medium), trigger: node.bypass)
        .sensoryFeedback(.impact(weight: .medium), trigger: lifted) { _, now in now }
        .accessibilityElement(children: .contain)
    }

    /// Fixed-size knob area; 1 knob is bigger and centred, 2 a centred row, 3 two over one, 4 a 2x2 grid.
    @ViewBuilder
    private func knobZone(_ ps: [ParamDesc]) -> some View {
        let big = ps.count == 1
        VStack(spacing: 0.45 * u) {
            ForEach(Array(stride(from: 0, to: ps.count, by: 2)), id: \.self) { row in
                HStack(spacing: 0) {
                    ForEach(Array(ps.dropFirst(row).prefix(2))) { p in
                        miniKnob(p, big: big).frame(maxWidth: .infinity)
                    }
                }
                .frame(width: ps.count - row == 1 && ps.count > 1 ? 4.2 * u : nil)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
    }

    private func miniKnob(_ p: ParamDesc, big: Bool) -> some View {
        VStack(spacing: 0.2 * u) {
            MiniKnob(n: normalized(store.numberValue(node, p), p), size: (big ? 3.2 : 2.25) * u)
            Text(shortLabel(p.label).uppercased())
                .font(.system(size: 0.62 * u, weight: .bold))
                .tracking(0.03 * u)
                .lineLimit(1).minimumScaleFactor(0.7)
                .foregroundStyle(.black.opacity(0.62))
        }
    }
}

func normalized(_ v: Double, _ p: ParamDesc) -> Double {
    guard p.max > p.min else { return 0 }
    if p.log, p.min > 0, v > 0 {
        return min(max((log(v) - log(p.min)) / (log(p.max) - log(p.min)), 0), 1)
    }
    return min(max((v - p.min) / (p.max - p.min), 0), 1)
}

struct MiniKnob: View {
    let n: Double
    let size: CGFloat
    var body: some View {
        Circle()
            .fill(RadialGradient(colors: [Color(white: 0.3), Color(white: 0.08)], center: UnitPoint(x: 0.35, y: 0.3), startRadius: 0, endRadius: size * 0.8))
            .frame(width: size, height: size)
            .overlay(Circle().stroke(.white.opacity(0.25), lineWidth: 0.7))
            .overlay {
                Capsule().fill(.white).frame(width: max(2, size * 0.085), height: size * 0.32).padding(.top, size * 0.1)
                    .frame(width: size, height: size, alignment: .top)
                    .rotationEffect(.degrees(-135 + 270 * n))
            }
            .shadow(color: .black.opacity(0.5), radius: size * 0.08, y: size * 0.06)
    }
}

private struct Footswitch: View {
    let u: CGFloat
    var body: some View {
        Circle()
            .fill(AngularGradient(colors: [Color(white: 0.55), Color(white: 0.95), Color(white: 0.55), Color(white: 0.95), Color(white: 0.55)], center: .center))
            .frame(width: 3.9 * u, height: 3.9 * u)
            .overlay(Circle().fill(Color(white: 0.85)).frame(width: 1.7 * u, height: 1.7 * u))
            .overlay(Circle().stroke(.black.opacity(0.3), lineWidth: 1))
            .shadow(color: .black.opacity(0.5), radius: 4, y: 3)
            // Generous touch target around the metal disc.
            .padding(8)
            .contentShape(Circle())
    }
}
