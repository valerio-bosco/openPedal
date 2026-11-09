import SwiftUI

/// "Voices" tab: the list of controls and the card of the selected one.
struct VoicesTab: View {
    let store: Store
    let model: DesignerModel
    @State private var picking = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(spacing: 8) {
                ForEach(model.controls) { c in voiceRow(c) }
                Button { withAnimation(.smooth) { model.addControl() } } label: {
                    Label("Add voice", systemImage: "plus").font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity).frame(height: 48)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(.white.opacity(0.25), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5])))
            }
            if let id = model.selectedControl, model.control(id) != nil {
                VoiceCard(store: store, model: model, id: id, picking: $picking)
            } else if model.controls.isEmpty {
                Text("A voice is a control on the pedal face (a knob, a switch or a selector). It drives one or more parameters of the blocks inside.")
                    .font(.footnote).foregroundStyle(.secondary).padding(.horizontal, 4)
            }
        }
        .sheet(isPresented: $picking) {
            if let id = model.selectedControl { TargetPicker(model: model, controlID: id) }
        }
    }

    private func voiceRow(_ c: ControlSpec) -> some View {
        let selected = model.selectedControl == c.id
        let bad = c.route.contains { !model.problems(of: $0, in: c).isEmpty }
        return Button { model.selectedControl = c.id } label: {
            HStack(spacing: 12) {
                Image(systemName: icon(c.kind))
                    .font(.system(size: 17, weight: .semibold)).foregroundStyle(.secondary)
                    .frame(width: 40, height: 40)
                    .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.black.opacity(0.3)))
                VStack(alignment: .leading, spacing: 2) {
                    Text(c.label.isEmpty ? c.id : c.label).font(.body.weight(.semibold))
                    Text("\(c.kind.rawValue) · \(c.route.count) target\(c.route.count == 1 ? "" : "s")")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if bad { Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Color(hex: 0xffb454)) }
                Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 14).frame(minHeight: 60)
            .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(.white.opacity(selected ? 0.14 : 0.06)))
            .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(.white.opacity(selected ? 0.5 : 0), lineWidth: 1.5))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private func icon(_ k: ControlType) -> String {
        switch k {
        case .knob: return "dial.medium"
        case .switch: return "switch.2"
        case .selector: return "list.number"
        }
    }
}

/// Everything about one voice: label, kind, default, positions, the parameters it drives.
private struct VoiceCard: View {
    let store: Store
    let model: DesignerModel
    let id: String
    @Binding var picking: Bool

    var body: some View {
        if let c = model.control(id) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 12) {
                    TextField("Label", text: Binding(
                        get: { model.control(id)?.label ?? "" },
                        set: { v in model.update(id) { $0.label = v } }))
                        .font(.system(size: 20, weight: .bold, design: .rounded))
                    Picker("Kind", selection: Binding(get: { c.kind }, set: { model.setKind(id, $0) })) {
                        Text("Knob").tag(ControlType.knob)
                        Text("Switch").tag(ControlType.switch)
                        Text("Selector").tag(ControlType.selector)
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 270)
                }

                defaultRow(c)
                if c.kind == .selector { positions(c) }

                Text("DRIVES").font(.caption2.weight(.semibold)).tracking(1).foregroundStyle(.tertiary)
                ForEach(c.route, id: \.target) { r in RouteRow(model: model, control: c, route: r) }
                Button { picking = true } label: {
                    Label("Add target", systemImage: "plus").font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity).frame(height: 48).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(.white.opacity(0.25), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5])))
                .disabled(model.nodes.isEmpty)

                Button(role: .destructive) { withAnimation(.smooth) { model.removeControl(id) } } label: {
                    Label("Delete voice", systemImage: "trash")
                }
                .buttonStyle(.glass)
            }
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(.white.opacity(0.06)))
        }
    }

    @ViewBuilder
    private func defaultRow(_ c: ControlSpec) -> some View {
        switch c.kind {
        case .knob:
            HStack {
                Text("Starts at").foregroundStyle(.secondary)
                Slider(value: Binding(
                    get: { model.defaultValue(c) },
                    set: { v in model.update(id) { $0.default = .number(v) }; model.values[id] = nil }), in: 0...1)
                Text(String(format: "%.0f%%", model.defaultValue(c) * 100)).font(.body.monospacedDigit()).frame(width: 48, alignment: .trailing)
            }
        case .switch:
            Toggle("Starts on", isOn: Binding(
                get: { model.defaultValue(c) >= 0.5 },
                set: { v in model.update(id) { $0.default = .boolean(v) }; model.values[id] = nil }))
        case .selector:
            HStack {
                Text("Starts at").foregroundStyle(.secondary)
                Picker("Starts at", selection: Binding(
                    get: { min(Int(model.defaultValue(c)), max((c.options?.count ?? 1) - 1, 0)) },
                    set: { v in model.update(id) { $0.default = .number(Double(v)) }; model.values[id] = nil })) {
                    ForEach(Array((c.options ?? []).enumerated()), id: \.offset) { i, o in Text(o).tag(i) }
                }
                .pickerStyle(.menu)
            }
        }
    }

    private func positions(_ c: ControlSpec) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("POSITIONS").font(.caption2.weight(.semibold)).tracking(1).foregroundStyle(.tertiary)
            ForEach(Array((c.options ?? []).enumerated()), id: \.offset) { i, _ in
                HStack(spacing: 10) {
                    Text("\(i + 1)").font(.subheadline.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22)
                    TextField("Name", text: Binding(
                        get: { model.control(id)?.options?[safe: i] ?? "" },
                        set: { v in model.update(id) { if $0.options?.indices.contains(i) == true { $0.options?[i] = v } } }))
                        .textFieldStyle(.plain)
                        .padding(.horizontal, 12).frame(height: 40)
                        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.white.opacity(0.1)))
                    Button { model.removeOption(id, at: i) } label: {
                        Image(systemName: "minus.circle").font(.title3).frame(width: 36, height: 36).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled((c.options?.count ?? 0) <= 2)
                    .opacity((c.options?.count ?? 0) <= 2 ? 0.3 : 1)
                    .accessibilityLabel("Remove position \(i + 1)")
                }
            }
            Button { model.addOption(id) } label: { Label("Add position", systemImage: "plus") }
                .buttonStyle(.glass)
        }
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

/// One parameter a voice drives, with the controls that depend on the voice kind.
private struct RouteRow: View {
    let model: DesignerModel
    let control: ControlSpec
    let route: RouteSpec

    var body: some View {
        let found = model.param(route.target)
        let issues = model.problems(of: route, in: control)
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(found?.desc.label ?? route.target).font(.body.weight(.semibold))
                    Text("\(model.nodeLabel(found?.node.id ?? "")) · \(route.target) · now \(model.currentText(control, route))")
                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Remove") { withAnimation(.smooth) { model.removeRoute(control.id, route.target) } }
                    .buttonStyle(.glass).controlSize(.small)
            }
            if let p = found?.desc {
                switch control.kind {
                case .knob: knob(p)
                case .switch: switchRoute(p, found?.node)
                case .selector: selector(p)
                }
            }
            ForEach(Array(issues.enumerated()), id: \.offset) { _, text in
                Label(text, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption).foregroundStyle(Color(hex: 0xffb454))
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(.black.opacity(0.22)))
    }

    private func edit(_ change: @escaping (inout RouteSpec) -> Void) {
        model.updateRoute(control.id, route.target, change)
    }

    // Knob: range and curve.
    @ViewBuilder
    private func knob(_ p: ParamDesc) -> some View {
        if p.tag == .number {
            let lo = route.min ?? p.min, hi = route.max ?? p.max
            let ordered = lo <= hi
            let low = ordered ? lo : hi, high = ordered ? hi : lo
            VStack(alignment: .leading, spacing: 10) {
                RangeSlider(
                    lo: Binding(get: { low }, set: { v in edit { $0.min = v; $0.max = max(high, v) } }),
                    hi: Binding(get: { high }, set: { v in edit { $0.max = v; $0.min = min(low, v) } }),
                    param: p, color: model.accent)
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 18) { fromTo(p, low, high); Spacer(minLength: 0); curves(p, low, high) }
                    VStack(alignment: .leading, spacing: 10) { fromTo(p, low, high); curves(p, low, high) }
                }
            }
        }
    }

    private func fromTo(_ p: ParamDesc, _ low: Double, _ high: Double) -> some View {
        HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                Text("From").font(.caption).foregroundStyle(.secondary)
                NumberEditor(value: Binding(get: { low }, set: { v in edit { $0.min = v; $0.max = max(high, v) } }), param: p)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("To").font(.caption).foregroundStyle(.secondary)
                NumberEditor(value: Binding(get: { high }, set: { v in edit { $0.max = v; $0.min = min(low, v) } }), param: p)
            }
        }
    }

    /// Curve buttons; log only exists when both ends are above zero.
    private func curves(_ p: ParamDesc, _ low: Double, _ high: Double) -> some View {
        let logOK = low > 0 && high > 0
        let current = route.map ?? .linear
        let chosen: MapKind = (current == .log && !logOK) ? .linear : current
        return HStack(spacing: 8) {
            ForEach(MapKind.allCases, id: \.self) { m in
                Button { edit { $0.map = m == .linear ? nil : m } } label: {
                    CurveThumb(map: m, lo: low, hi: high)
                        .foregroundStyle(chosen == m ? model.accent : .secondary)
                        .frame(width: 52, height: 40)
                        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.white.opacity(chosen == m ? 0.16 : 0.06)))
                        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(chosen == m ? model.accent : .clear, lineWidth: 1.5))
                }
                .buttonStyle(.plain)
                .disabled(m == .log && !logOK)
                .opacity(m == .log && !logOK ? 0.3 : 1)
                .accessibilityLabel(m.rawValue.capitalized + " curve")
                .accessibilityAddTraits(chosen == m ? .isSelected : [])
            }
        }
    }

    // Switch: a value for off and for on, and "Add mode" for numbers.
    @ViewBuilder
    private func switchRoute(_ p: ParamDesc, _ node: NodeSpec?) -> some View {
        let canAdd = p.tag == .number && !(node?.type.hasPrefix(OpenPedalProtocol.pedalPrefix) ?? false)
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 24) {
                valueCell("Off", p, route.off) { v in edit { $0.off = v } }
                valueCell(route.mode == .add ? "On (adds)" : "On", p, route.on) { v in edit { $0.on = v } }
            }
            if canAdd {
                Toggle(isOn: Binding(get: { route.mode == .add }, set: { v in edit { $0.mode = v ? .add : nil } })) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Add mode")
                        Text("On adds to what the other voices set, instead of replacing it.").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    // Selector: one value per position.
    @ViewBuilder
    private func selector(_ p: ParamDesc) -> some View {
        let options = control.options ?? []
        let values = route.values ?? []
        WrapLayout(hSpacing: 22, vSpacing: 10) {
            ForEach(Array(options.enumerated()), id: \.offset) { i, name in
                valueCell(name.isEmpty ? "Position \(i + 1)" : name, p, values[safe: i]) { v in
                    edit { r in
                        var all = r.values ?? []
                        while all.count < options.count { all.append(v) }
                        all[i] = v
                        r.values = all
                    }
                }
            }
        }
    }

    /// A value editor for a parameter: a number field for numbers, a menu for choices.
    @ViewBuilder
    private func valueCell(_ title: String, _ p: ParamDesc, _ value: ParamValue?, _ set: @escaping (ParamValue) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            if p.tag == .choice {
                Menu {
                    ForEach(p.options, id: \.self) { o in Button(o) { set(.string(o)) } }
                } label: {
                    HStack(spacing: 6) {
                        Text(model.valueText(value, p)).font(.subheadline.weight(.semibold))
                        Image(systemName: "chevron.up.chevron.down").font(.caption2).foregroundStyle(.secondary)
                    }
                    .frame(minWidth: 90).padding(.horizontal, 10).frame(height: 34)
                    .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.white.opacity(0.1)))
                }
            } else {
                // Add-mode offsets may be negative or larger than the parameter, so the range is wider than the parameter's.
                let span = p.max - p.min
                NumberEditor(
                    value: Binding(get: { DesignerModel.number(value) ?? p.def }, set: { set(.number($0)) }),
                    range: (route.mode == .add ? -span : p.min)...(route.mode == .add ? span : p.max),
                    step: p.step, unit: p.unit)
            }
        }
    }
}

/// Glass list of the parameters a voice can drive, grouped by inner block.
struct TargetPicker: View {
    let model: DesignerModel
    let controlID: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let control = model.control(controlID)
        let knob = control?.kind == .knob
        NavigationStack {
            List {
                ForEach(model.nodes) { node in
                    let params = (model.items(node)?.params ?? []).filter { p in
                        p.tag != .file && (!knob || p.tag == .number)
                    }
                    if !params.isEmpty {
                        Section("\(model.nodeLabel(node.id)) · \(node.id)") {
                            ForEach(params) { p in
                                let target = "\(node.id).\(p.id)"
                                let taken = control?.route.contains { $0.target == target } ?? false
                                Button {
                                    model.addRoute(controlID, target: target)
                                    dismiss()
                                } label: {
                                    HStack {
                                        Text(p.label)
                                        Spacer()
                                        Text(current(node, p)).font(.caption.monospaced()).foregroundStyle(.secondary)
                                    }
                                }
                                .disabled(taken)
                            }
                        }
                    }
                }
            }
            .navigationTitle(knob ? "Add target (numbers)" : "Add target")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private func current(_ node: NodeSpec, _ p: ParamDesc) -> String {
        if p.tag == .choice { return p.options[safe: model.nodeChoice(node, p)] ?? "" }
        return formatValue(model.nodeNumber(node, p), p)
    }
}
