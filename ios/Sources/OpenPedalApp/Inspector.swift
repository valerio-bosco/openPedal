import SwiftUI

/// Floating glass panel with the controls of the selected node.
struct InspectorView: View {
    let store: Store

    var body: some View {
        Group {
            if let node = store.selected {
                content(node)
            } else {
                Text("Select a pedal")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 120)
            }
        }
        .padding(20)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }

    @ViewBuilder
    private func content(_ node: NodeSpec) -> some View {
        let item = store.item(for: node)
        let canBypass = node.type != "input" && node.type != "output"
        HStack(alignment: .center, spacing: 24) {
            VStack(alignment: .leading, spacing: 8) {
                Text(item?.label ?? node.type).font(.title2.weight(.bold)).lineLimit(1)
                Text(kindLine(node, item)).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
                if canBypass {
                    Toggle(isOn: Binding(get: { !node.bypass }, set: { store.setBypass(node.id, !$0) })) {
                        Text(node.bypass ? "Bypassed" : "Active").font(.subheadline.weight(.semibold))
                    }
                    .fixedSize()
                    .padding(.top, 4)
                }
                if !store.canEditChain {
                    Label("Parallel routing: edit on the desktop", systemImage: "info.circle")
                        .font(.caption).foregroundStyle(.secondary).padding(.top, 4)
                }
            }
            .frame(width: 190, alignment: .leading)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 22) {
                    ForEach(item?.params ?? []) { p in
                        switch p.kind {
                        case .number:
                            KnobView(
                                label: p.label,
                                n: normalized(store.numberValue(node, p), p),
                                valueText: formatValue(store.numberValue(node, p), p),
                                detent: detent(store.numberValue(node, p), p),
                                onBegin: { store.beginEdit() },
                                onChange: { store.setNumber(node, p, value(forNormalized: $0, p)) },
                                onReset: { store.setDefault(node, p) }
                            )
                        case .choice:
                            VStack(spacing: 8) {
                                Picker(p.label, selection: Binding(
                                    get: { store.choiceIndex(node, p) },
                                    set: { store.setChoice(node, p, index: $0) })
                                ) {
                                    ForEach(Array(p.options.enumerated()), id: \.offset) { i, o in Text(o).tag(i) }
                                }
                                .pickerStyle(.segmented)
                                .frame(width: CGFloat(max(p.options.count, 2)) * 76)
                                Text(p.label).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                            }
                            .padding(.top, 28)
                        case .file:
                            VStack(spacing: 6) {
                                Text(fileName(node, p)).font(.callout.weight(.semibold)).lineLimit(1).frame(maxWidth: 160)
                                Text(p.label + " (edit on the desktop)").font(.caption).foregroundStyle(.secondary)
                            }
                            .padding(.top, 36)
                        }
                    }
                }
                .padding(.horizontal, 4)
            }
        }
    }

    private func kindLine(_ node: NodeSpec, _ item: CatalogItem?) -> String {
        var parts = [item?.category.capitalized ?? "Node"]
        if item?.composite == true { parts.append("composite") }
        parts.append(node.id)
        return parts.joined(separator: " · ")
    }

    private func fileName(_ node: NodeSpec, _ p: ParamDesc) -> String {
        if case .string(let s) = node.params[p.id], !s.isEmpty { return (s as NSString).lastPathComponent }
        return "built-in"
    }

    private func detent(_ v: Double, _ p: ParamDesc) -> Int {
        p.step > 0 ? Int(((v - p.min) / p.step).rounded()) : Int((normalized(v, p) * 24).rounded())
    }
}

/// Parameter value for a knob position 0...1: log scale and `step` honoured.
func value(forNormalized n: Double, _ p: ParamDesc) -> Double {
    let n = min(max(n, 0), 1)
    var v: Double
    if p.log, p.min > 0 { v = exp(log(p.min) + n * (log(p.max) - log(p.min))) }
    else { v = p.min + n * (p.max - p.min) }
    if p.step > 0 { v = p.min + ((v - p.min) / p.step).rounded() * p.step }
    return min(max(v, p.min), p.max)
}

/// Big knob: drag up/right to turn up, down/left to turn down; double tap resets.
struct KnobView: View {
    let label: String
    let n: Double
    let valueText: String
    let detent: Int
    let onBegin: () -> Void
    let onChange: (Double) -> Void
    let onReset: () -> Void

    @State private var startN: Double?
    private static let size: CGFloat = 92
    /// Points of drag for the whole range.
    private static let travel: Double = 220

    var body: some View {
        VStack(spacing: 6) {
            ZStack {
                Circle().trim(from: 0, to: 0.75)
                    .stroke(.white.opacity(0.15), style: StrokeStyle(lineWidth: 5, lineCap: .round))
                    .rotationEffect(.degrees(135))
                Circle().trim(from: 0, to: 0.75 * n)
                    .stroke(.tint, style: StrokeStyle(lineWidth: 5, lineCap: .round))
                    .rotationEffect(.degrees(135))
                Circle()
                    .fill(RadialGradient(colors: [Color(white: 0.34), Color(white: 0.07)], center: UnitPoint(x: 0.35, y: 0.3), startRadius: 0, endRadius: 60))
                    .overlay(Circle().stroke(.white.opacity(0.3), lineWidth: 0.8))
                    .padding(10)
                    .shadow(color: .black.opacity(0.5), radius: 5, y: 3)
                Capsule().fill(.white).frame(width: 4, height: 18).padding(.top, 14)
                    .frame(width: Self.size, height: Self.size, alignment: .top)
                    .rotationEffect(.degrees(-135 + 270 * n))
            }
            .frame(width: Self.size, height: Self.size)
            .contentShape(Circle())
            .gesture(
                DragGesture(minimumDistance: 1)
                    .onChanged { g in
                        if startN == nil { startN = n; onBegin() }
                        let delta = (-g.translation.height + g.translation.width) / Self.travel
                        onChange((startN ?? n) + delta)
                    }
                    .onEnded { _ in startN = nil }
            )
            .onTapGesture(count: 2, perform: onReset)
            .sensoryFeedback(.selection, trigger: detent)

            Text(valueText).font(.callout.monospacedDigit().weight(.semibold))
            Text(label).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
        .frame(width: 100)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(valueText)
        .accessibilityAdjustableAction { dir in
            switch dir {
            case .increment: onChange(n + 0.05)
            case .decrement: onChange(n - 0.05)
            @unknown default: break
            }
        }
    }
}
