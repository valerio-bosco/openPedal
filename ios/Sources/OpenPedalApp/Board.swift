import SwiftUI

/// Full-bleed horizontal pedalboard: input jack, pedals in signal order, output jack, "+".
struct BoardView: View {
    let store: Store
    @Binding var showAdd: Bool

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 22) {
                    ForEach(store.chain.nodes) { node in
                        Group {
                            if node.type == "input" || node.type == "output" {
                                JackView(store: store, node: node)
                            } else {
                                PedalView(store: store, node: node)
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
                .padding(.horizontal, 80)
                .padding(.vertical, 50)
                .frame(minHeight: 340)
                .background(alignment: .center) { Wire() }
            }
            .scrollClipDisabled()
            .onChange(of: store.selectedID) { _, id in
                if let id { withAnimation(.smooth) { proxy.scrollTo(id, anchor: .center) } }
            }
        }
    }
}

/// The patch cable running behind the pedals.
private struct Wire: View {
    var body: some View {
        Capsule()
            .fill(LinearGradient(colors: [Color(white: 0.1), Color(white: 0.22), Color(white: 0.1)], startPoint: .leading, endPoint: .trailing))
            .frame(height: 4)
            .padding(.horizontal, 60)
            .shadow(color: .white.opacity(0.06), radius: 0, y: 1)
    }
}

/// Input ("Guitar") and output ("Amp") ends of the chain.
struct JackView: View {
    let store: Store
    let node: NodeSpec
    var body: some View {
        let isSel = store.selectedID == node.id
        let label = node.type == "input" ? "Guitar" : "Amp"
        VStack(spacing: 6) {
            Image(systemName: node.type == "input" ? "guitars" : "hifispeaker.fill")
                .font(.system(size: 26))
            Text(label).font(.caption.weight(.semibold))
        }
        .foregroundStyle(.white.opacity(isSel ? 1 : 0.7))
        .frame(width: 84, height: 84)
        .glassEffect(isSel ? .regular.tint(.white.opacity(0.18)) : .regular, in: .circle)
        .onTapGesture { store.selectedID = node.id }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityAddTraits(.isButton)
    }
}

struct PedalView: View {
    let store: Store
    let node: NodeSpec

    private static let width: CGFloat = 150
    private static let height: CGFloat = 250

    var body: some View {
        let item = store.item(for: node)
        let color = enamel(for: node.type, category: item?.category)
        let isSel = store.selectedID == node.id
        let numbers = (item?.params ?? []).filter { $0.kind == .number }.prefix(4)
        let on = !node.bypass

        VStack(spacing: 10) {
            Text((item?.label ?? node.type).uppercased())
                .font(.system(size: 11, weight: .heavy, design: .rounded))
                .tracking(1)
                .lineLimit(1).minimumScaleFactor(0.6)
                .foregroundStyle(.black.opacity(0.65))
                .padding(.top, 14)

            // Mini knobs reflecting the current values.
            Grid(horizontalSpacing: 14, verticalSpacing: 8) {
                ForEach(Array(stride(from: 0, to: numbers.count, by: 2)), id: \.self) { row in
                    GridRow {
                        ForEach(Array(numbers.dropFirst(row).prefix(2))) { p in
                            VStack(spacing: 2) {
                                MiniKnob(n: normalized(store.numberValue(node, p), p))
                                Text(p.label).font(.system(size: 8, weight: .bold)).lineLimit(1)
                                    .foregroundStyle(.black.opacity(0.6))
                                    .frame(maxWidth: 56)
                            }
                        }
                    }
                }
            }
            .frame(minHeight: 100, alignment: .top)

            Circle()
                .fill(on ? Color(hex: 0xff3b30) : Color(hex: 0x4a0d0d))
                .frame(width: 11, height: 11)
                .shadow(color: on ? Color(hex: 0xff3b30) : .clear, radius: 6)
                .overlay(Circle().stroke(.black.opacity(0.35), lineWidth: 0.5))

            Button { store.setBypass(node.id, on) } label: { Footswitch() }
                .buttonStyle(.plain)
                .accessibilityLabel(on ? "Bypass \(item?.label ?? node.type)" : "Enable \(item?.label ?? node.type)")
            Spacer(minLength: 0)
        }
        .frame(width: Self.width, height: Self.height)
        .background {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(LinearGradient(colors: [color.opacity(0.95), color.mix(with: .black, by: 0.35)], startPoint: .top, endPoint: .bottom))
                .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous)
                    .stroke(LinearGradient(colors: [.white.opacity(0.55), .black.opacity(0.3)], startPoint: .top, endPoint: .bottom), lineWidth: 1.2))
                .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous)
                    .stroke(.white, lineWidth: isSel ? 3 : 0))
                .shadow(color: isSel ? color.opacity(0.7) : .black.opacity(0.5), radius: isSel ? 28 : 12, y: isSel ? 14 : 8)
        }
        .saturation(on ? 1 : 0.35)
        .brightness(on ? 0 : -0.12)
        .scaleEffect(isSel ? 1.06 : 1)
        .offset(y: isSel ? -14 : 0)
        .animation(.spring(response: 0.35, dampingFraction: 0.7), value: isSel)
        .animation(.easeOut(duration: 0.2), value: on)
        .contentShape(RoundedRectangle(cornerRadius: 20))
        .onTapGesture { store.selectedID = node.id }
        .contextMenu {
            if store.canEditChain {
                Button("Move left", systemImage: "arrow.left") { store.movePedal(node.id, by: -1) }
                Button("Move right", systemImage: "arrow.right") { store.movePedal(node.id, by: 1) }
                Button("Remove", systemImage: "trash", role: .destructive) { store.removePedal(node.id) }
            }
        }
        .sensoryFeedback(.impact(weight: .medium), trigger: node.bypass)
        .accessibilityElement(children: .contain)
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
    var body: some View {
        Circle()
            .fill(RadialGradient(colors: [Color(white: 0.3), Color(white: 0.08)], center: UnitPoint(x: 0.35, y: 0.3), startRadius: 0, endRadius: 28))
            .frame(width: 36, height: 36)
            .overlay(Circle().stroke(.white.opacity(0.25), lineWidth: 0.7))
            .overlay {
                Capsule().fill(.white).frame(width: 3, height: 11).padding(.top, 3)
                    .frame(width: 36, height: 36, alignment: .top)
                    .rotationEffect(.degrees(-135 + 270 * n))
            }
            .shadow(color: .black.opacity(0.5), radius: 3, y: 2)
    }
}

private struct Footswitch: View {
    var body: some View {
        Circle()
            .fill(AngularGradient(colors: [Color(white: 0.55), Color(white: 0.95), Color(white: 0.55), Color(white: 0.95), Color(white: 0.55)], center: .center))
            .frame(width: 54, height: 54)
            .overlay(Circle().fill(Color(white: 0.85)).frame(width: 24, height: 24))
            .overlay(Circle().stroke(.black.opacity(0.3), lineWidth: 1))
            .shadow(color: .black.opacity(0.5), radius: 4, y: 3)
            .frame(width: 66, height: 66)
            .contentShape(Circle())
    }
}
