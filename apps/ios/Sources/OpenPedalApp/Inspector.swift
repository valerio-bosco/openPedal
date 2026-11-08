import SwiftUI

enum InspectorDetent {
    static let compact: CGFloat = 252
    static let expanded: CGFloat = 540
    static var mid: CGFloat { (compact + expanded) / 2 }
}

/// Floating glass panel with the controls of the selected node. Two detents: compact (one scrolling
/// row) and expanded (modes + grid of controls), resized by the grabber or the "All controls" button.
struct InspectorView: View {
    let store: Store
    @Binding var expanded: Bool
    /// Height while the grabber is being dragged; nil otherwise.
    @Binding var liveHeight: CGFloat?
    /// Animated panel height (the glass window).
    let height: CGFloat
    /// Height of the current detent: the content is laid out at this size at once and revealed by the window.
    let detentHeight: CGFloat

    @State private var dragStart: CGFloat?

    private static let snap = Animation.spring(response: 0.4, dampingFraction: 0.85)

    var body: some View {
        VStack(spacing: 0) {
            grabber
            if let model = store.selectedModel {
                content(model)
            } else {
                Text("Select a pedal")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        // Content at its final size, not animated: only the glass window below changes size per frame.
        .frame(height: detentHeight, alignment: .top)
        .transaction(value: detentHeight) { $0.animation = nil }
        .frame(height: height, alignment: .top)
        .clipShape(RoundedRectangle(cornerRadius: 32, style: .continuous))
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }

    private var grabber: some View {
        Capsule().fill(.white.opacity(0.32)).frame(width: 40, height: 5)
            .frame(maxWidth: .infinity).frame(height: 26)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .global)
                    .onChanged { g in
                        if dragStart == nil { dragStart = height }
                        let h = (dragStart ?? height) - g.translation.height
                        liveHeight = min(max(h, InspectorDetent.compact - 24), InspectorDetent.expanded + 24)
                    }
                    .onEnded { g in
                        let projected = (dragStart ?? height) - g.predictedEndTranslation.height
                        dragStart = nil
                        withAnimation(Self.snap) {
                            expanded = projected > InspectorDetent.mid
                            liveHeight = nil
                        }
                    }
            )
            .accessibilityLabel(expanded ? "Collapse controls" : "Expand controls")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { withAnimation(Self.snap) { expanded.toggle() } }
    }

    // MARK: header and body

    @ViewBuilder
    private func content(_ model: NodeModel) -> some View {
        // Reads only structure (type, catalog): value changes redraw the cells, not this panel.
        let item = store.item(kind: model.type)
        let accent = enamel(for: model.type, category: item?.category.rawValue)
        let params = item?.params ?? []
        VStack(spacing: 0) {
            InspectorHeader(store: store, model: model, item: item, accent: accent, count: params.count, expanded: $expanded)
            if params.isEmpty {
                Text("No controls").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if expanded {
                expandedBody(model, params, accent)
            } else {
                compactBody(model, params, accent)
            }
        }
    }

    private func compactBody(_ node: NodeModel, _ params: [ParamDesc], _ accent: Color) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                cells(node, params.filter { $0.tag == .choice }, accent)
                cells(node, params.filter { $0.tag != .choice }, accent)
            }
            .padding(.leading, 20).padding(.trailing, 60)
            .frame(maxHeight: .infinity, alignment: .top)
        }
        .mask {
            HStack(spacing: 0) {
                Color.black
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing).frame(width: 60)
            }
        }
        .padding(.bottom, 14)
    }

    private func expandedBody(_ node: NodeModel, _ params: [ParamDesc], _ accent: Color) -> some View {
        let modes = params.filter { $0.tag == .choice }
        let controls = params.filter { $0.tag != .choice }
        return ScrollView(.vertical, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 14) {
                if !modes.isEmpty {
                    section("Modes") {
                        WrapLayout(hSpacing: 22, vSpacing: 10) { cells(node, modes, accent) }
                    }
                }
                if !controls.isEmpty {
                    section("Controls") {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 112, maximum: 112), spacing: 4)],
                                  alignment: .leading, spacing: 6) {
                            cells(node, controls, accent)
                        }
                    }
                }
            }
            .padding(.horizontal, 20).padding(.top, 4).padding(.bottom, 28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func section<C: View>(_ title: String, @ViewBuilder _ body: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title.uppercased()).font(.caption.weight(.semibold)).tracking(1).foregroundStyle(.tertiary).padding(.leading, 4)
            body()
        }
    }

    @ViewBuilder
    private func cells(_ model: NodeModel, _ params: [ParamDesc], _ accent: Color) -> some View {
        ForEach(params) { p in ParamCell(store: store, model: model, p: p, accent: accent) }
    }
}

func knobDetent(_ v: Double, _ p: ParamDesc) -> Int {
    p.step > 0 ? Int(((v - p.min) / p.step).rounded()) : Int((normalized(v, p) * 24).rounded())
}

/// Header row: swatch, name, kind, Reset, All controls, chain actions, Active toggle.
/// Separate view so only it observes the bypass flag.
private struct InspectorHeader: View {
    let store: Store
    let model: NodeModel
    let item: CatalogItem?
    let accent: Color
    let count: Int
    @Binding var expanded: Bool

    private static let snap = Animation.spring(response: 0.4, dampingFraction: 0.85)

    var body: some View {
        let isPedal = model.type != "input" && model.type != "output"
        HStack(spacing: 12) {
            Circle().fill(accent).frame(width: 12, height: 12).shadow(color: accent, radius: 5)
            Text(item?.label ?? model.type)
                .font(.system(size: 22, weight: .bold, design: .rounded)).lineLimit(1)
            Text(kindLine).font(.callout).foregroundStyle(.tertiary).lineLimit(1)
            if !store.canEditChain {
                Label("Parallel routing: edit on the desktop", systemImage: "info.circle")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 8)
            if count > 0 {
                Button { store.resetParams(model.id) } label: {
                    Label("Reset", systemImage: "arrow.counterclockwise").font(.subheadline.weight(.semibold))
                }
                .buttonStyle(.glass)
                Button {
                    withAnimation(Self.snap) { expanded.toggle() }
                } label: {
                    HStack(spacing: 6) {
                        Text(expanded ? "Less" : "All controls · \(count)").font(.subheadline.weight(.semibold))
                        Image(systemName: "chevron.up").font(.caption.weight(.bold))
                            .rotationEffect(.degrees(expanded ? 180 : 0))
                            .animation(Self.snap, value: expanded)
                    }
                }
                .buttonStyle(.glass)
            }
            if isPedal && store.canEditChain {
                Menu {
                    Button("Move left", systemImage: "arrow.left") { store.movePedal(model.id, by: -1) }
                    Button("Move right", systemImage: "arrow.right") { store.movePedal(model.id, by: 1) }
                    Divider()
                    Button("Remove", systemImage: "trash", role: .destructive) { store.removePedal(model.id) }
                } label: {
                    Image(systemName: "ellipsis").font(.body.weight(.semibold)).frame(width: 22, height: 22)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .accessibilityLabel("Pedal actions")
            }
            if isPedal {
                Text("Active").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary).padding(.leading, 6)
                Toggle("Active", isOn: Binding(get: { !model.spec.isBypassed }, set: { store.setBypass(model.id, !$0) }))
                    .labelsHidden()
                    .tint(.green)
            }
        }
        .frame(height: 44)
        .padding(.leading, 24).padding(.trailing, 16)
        .padding(.bottom, 6)
    }

    private var kindLine: String {
        var parts = [item?.category.rawValue.capitalized ?? "Node"]
        if item?.composite == true { parts.append("composite") }
        parts.append(model.id)
        return parts.joined(separator: " · ")
    }
}

/// One control of the selected node; observes only that node's values. The knob inside is
/// Equatable, so the other knobs of the pedal skip their body while one is turned.
private struct ParamCell: View {
    let store: Store
    let model: NodeModel
    let p: ParamDesc
    let accent: Color

    var body: some View {
        let node = model.spec
        switch p.tag {
        case .number:
            let v = store.numberValue(node, p)
            KnobView(
                label: p.label,
                accent: accent,
                n: normalized(v, p),
                valueText: formatValue(v, p),
                detent: knobDetent(v, p),
                onBegin: { store.beginEdit() },
                onChange: { store.setNumber(model.id, p, value(forNormalized: $0, p)) },
                onReset: { store.setDefault(model.id, p) }
            )
            .equatable()
        case .choice:
            ControlCell(width: CGFloat(max(p.options.count, 2)) * 76 + 16, value: "", name: p.label) {
                Picker(p.label, selection: Binding(
                    get: { store.choiceIndex(model.spec, p) },
                    set: { store.setChoice(model.id, p, index: $0) })
                ) {
                    ForEach(Array(p.options.enumerated()), id: \.offset) { i, o in Text(o.capitalized).tag(i) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: CGFloat(max(p.options.count, 2)) * 76)
            }
        case .file:
            ControlCell(value: fileName(node), name: p.label) {
                VStack(spacing: 4) {
                    Image(systemName: "doc.waveform").font(.title2)
                    Text("Desktop only").font(.caption2)
                }
                .foregroundStyle(.secondary)
            }
            .accessibilityHint("Edit on the desktop")
        }
    }

    private func fileName(_ node: NodeSpec) -> String {
        if case .string(let s) = node.params[p.id], !s.isEmpty { return (s as NSString).lastPathComponent }
        return "built-in"
    }
}

/// Wraps its subviews onto several rows (the "Modes" section).
struct WrapLayout: Layout {
    var hSpacing: CGFloat
    var vSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(proposal.width ?? .infinity, subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let r = arrange(bounds.width, subviews)
        for (s, p) in zip(subviews, r.origins) {
            s.place(at: CGPoint(x: bounds.minX + p.x, y: bounds.minY + p.y), proposal: .unspecified)
        }
    }

    private func arrange(_ width: CGFloat, _ subviews: Subviews) -> (size: CGSize, origins: [CGPoint]) {
        var origins: [CGPoint] = []
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0, maxW: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > 0, x + sz.width > width { x = 0; y += rowH + vSpacing; rowH = 0 }
            origins.append(CGPoint(x: x, y: y))
            x += sz.width + hSpacing
            rowH = max(rowH, sz.height)
            maxW = max(maxW, x - hSpacing)
        }
        return (CGSize(width: maxW, height: y + rowH), origins)
    }
}

/// Every control is a cell with the same three rows (control box 84, value 22, name 18),
/// so labels line up across knobs, pickers and file cells.
struct ControlCell<Box: View>: View {
    var width: CGFloat = 112
    let value: String
    let name: String
    var valueColor: Color = .primary
    var highlight = false
    @ViewBuilder let box: Box

    var body: some View {
        VStack(spacing: 0) {
            box.frame(height: 84)
            Text(value.isEmpty ? " " : value)
                .font(.system(size: 15, weight: .semibold).monospacedDigit())
                .foregroundStyle(valueColor)
                .lineLimit(1).minimumScaleFactor(0.8)
                .frame(maxWidth: width - 8, minHeight: 22, maxHeight: 22)
            Text(name)
                .font(.system(size: 12, weight: .medium)).foregroundStyle(.secondary)
                .lineLimit(1)
                .frame(maxWidth: width - 8, minHeight: 18, maxHeight: 18)
        }
        .frame(width: width, height: 138)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(.white.opacity(highlight ? 0.06 : 0)))
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

/// Knob: drag up/right to turn up, down/left to turn down; double tap resets.
struct KnobView: View, Equatable {
    let label: String
    let accent: Color
    let n: Double
    let valueText: String
    let detent: Int
    let onBegin: () -> Void
    let onChange: (Double) -> Void
    let onReset: () -> Void

    @State private var startN: Double?

    /// Closures are ignored: they only forward to the store with the node id, which never changes.
    nonisolated static func == (a: KnobView, b: KnobView) -> Bool {
        a.label == b.label && a.accent == b.accent && a.n == b.n && a.valueText == b.valueText && a.detent == b.detent
    }

    private static let size: CGFloat = 80
    /// Points of drag for the whole range.
    private static let travel: Double = 220

    var body: some View {
        ControlCell(value: valueText, name: label, valueColor: startN == nil ? .primary : accent, highlight: startN != nil) {
            knob
        }
        .contentShape(Rectangle())
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

    /// 80 pt, arc and cap fully inside the bounds.
    private var knob: some View {
        ZStack {
            Circle().trim(from: 0, to: 0.75)
                .stroke(.white.opacity(0.12), style: StrokeStyle(lineWidth: 5, lineCap: .round))
                .rotationEffect(.degrees(135))
                .padding(6)
            Circle().trim(from: 0, to: 0.75 * n)
                .stroke(accent, style: StrokeStyle(lineWidth: 5, lineCap: .round))
                .rotationEffect(.degrees(135))
                .padding(6)
            Circle()
                .fill(RadialGradient(colors: [Color(white: 0.34), Color(white: 0.1)], center: UnitPoint(x: 0.35, y: 0.3), startRadius: 0, endRadius: 40))
                .overlay(Circle().stroke(.white.opacity(0.18), lineWidth: 1))
                .frame(width: 48, height: 48)
                .shadow(color: .black.opacity(0.5), radius: 4, y: 2)
            Capsule().fill(.white).frame(width: 3, height: 10).padding(.top, 20)
                .frame(width: Self.size, height: Self.size, alignment: .top)
                .rotationEffect(.degrees(-135 + 270 * n))
        }
        .frame(width: Self.size, height: Self.size)
    }
}
