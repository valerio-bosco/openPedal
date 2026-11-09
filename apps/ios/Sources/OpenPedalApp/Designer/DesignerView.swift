import SwiftUI

/// Full-screen pedal designer (docs/PLAN.md §12, mockup "iPad"): top bar, a left glass panel with the
/// name, colour, live preview and response graph, and a right glass panel with Voices / Inside.
struct DesignerView: View {
    let store: Store
    let session: DesignerSession
    @Environment(\.dismiss) private var dismiss
    @State private var saving = false
    @State private var confirmDiscard = false

    private var model: DesignerModel { session.model }

    var body: some View {
        ZStack(alignment: .top) {
            Ambient(tint: model.accent)
            GeometryReader { geo in
                let wide = geo.size.width >= 900
                VStack(spacing: 14) {
                    topBar
                    if wide {
                        HStack(alignment: .top, spacing: 16) {
                            DesignerLeftPanel(store: store, model: model, compact: false).frame(width: 400)
                            DesignerRightPanel(store: store, model: model)
                        }
                    } else {
                        DesignerLeftPanel(store: store, model: model, compact: true).frame(height: 330)
                        DesignerRightPanel(store: store, model: model)
                    }
                }
                .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 16)
            }
            if let text = store.banner {
                Label(text, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout.weight(.semibold))
                    .padding(.horizontal, 18).padding(.vertical, 10)
                    .glassEffect(.regular.tint(.red.opacity(0.35)), in: .capsule)
                    .padding(.top, 70)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.smooth, value: store.banner)
        .preferredColorScheme(.dark)
        // Debounced (~80 ms), latest wins: a new key cancels the pending request. Only valid drafts are sent.
        .task(id: model.analysisKey) {
            guard model.structureProblems.isEmpty, !model.nodes.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(80))
            if Task.isCancelled { return }
            store.analyze(model.pedalFile(), values: model.analysisValues)
        }
        .onAppear {
            store.analysis.freq = []
            store.analysis.db = []
        }
        .confirmationDialog("Discard this pedal?", isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard changes", role: .destructive) { dismiss() }
            Button("Keep editing", role: .cancel) {}
        }
    }

    private var topBar: some View {
        let problems = model.problems
        return GlassEffectContainer(spacing: 12) {
            HStack(spacing: 12) {
                Button {
                    if model.hasChanges { confirmDiscard = true } else { dismiss() }
                } label: {
                    Text("Cancel").font(.body.weight(.semibold)).padding(.horizontal, 6)
                }
                .buttonStyle(.glass)
                .controlSize(.large)

                Spacer()
                VStack(spacing: 1) {
                    Text(model.isNew ? "New pedal" : "Edit pedal").font(.headline)
                    if !session.mergeIDs.isEmpty {
                        Text("Replaces \(session.mergeIDs.count) pedal\(session.mergeIDs.count == 1 ? "" : "s") on the board")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer()

                Button(action: save) {
                    Text("Save").font(.body.weight(.semibold)).padding(.horizontal, 10)
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .disabled(!problems.isEmpty || saving)
                .accessibilityHint(problems.first ?? "")
            }
        }
        .frame(height: 52)
    }

    private func save() {
        guard model.problems.isEmpty, let id = model.newPedalID() else { return }
        saving = true
        let ids = session.mergeIDs
        store.savePedal(id: id, pedal: model.pedalFile()) { ok in
            saving = false
            guard ok else { return }
            if !ids.isEmpty { store.mergeIntoPedal(ids: ids, pedalID: id) }
            dismiss()
        }
    }
}

// MARK: - left panel

private struct DesignerLeftPanel: View {
    let store: Store
    let model: DesignerModel
    let compact: Bool

    var body: some View {
        Group {
            if compact {
                HStack(spacing: 16) {
                    PedalPreview(model: model).fixedSize()
                    VStack(spacing: 12) {
                        identity
                        graph
                    }
                }
            } else {
                VStack(spacing: 14) {
                    identity
                    PedalPreview(model: model).fixedSize()
                    graph
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }

    private var identity: some View {
        @Bindable var model = model
        return VStack(spacing: 12) {
            TextField("Pedal name", text: $model.name)
                .font(.system(size: 17, weight: .semibold))
                .textFieldStyle(.plain)
                .padding(.horizontal, 14).frame(height: 44)
                .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(.white.opacity(0.1)))
                .submitLabel(.done)
            HStack(spacing: 8) {
                ForEach(designerColors, id: \.self) { hex in
                    Button { model.color = hex } label: {
                        Circle().fill(Color(css: hex) ?? .gray)
                            .frame(width: 28, height: 28)
                            .overlay(Circle().strokeBorder(.white, lineWidth: model.color == hex ? 3 : 0))
                            .padding(3)
                            .contentShape(Circle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Colour \(hex)")
                    .accessibilityAddTraits(model.color == hex ? .isSelected : [])
                }
            }
            Menu {
                Picker("Category", selection: $model.category) {
                    ForEach(Category.allCases.filter { $0 != .io }, id: \.self) { c in Text(c.rawValue.capitalized).tag(c) }
                }
            } label: {
                HStack(spacing: 6) {
                    Text("Category").foregroundStyle(.secondary)
                    Text(model.category.rawValue.capitalized)
                    Image(systemName: "chevron.up.chevron.down").font(.caption2).foregroundStyle(.secondary)
                }
                .font(.subheadline.weight(.medium))
            }
            .buttonStyle(.glass)
        }
    }

    private var graph: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("FREQUENCY RESPONSE").font(.caption2.weight(.semibold)).tracking(1).foregroundStyle(.tertiary)
            ResponseGraph(model: store.analysis, color: model.accent)
                .frame(maxWidth: .infinity, minHeight: 110, maxHeight: .infinity)
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(.black.opacity(0.25)))
    }
}

// MARK: - live preview

/// The pedal as it will look: knobs are dragged, switches and selectors tapped. Moves only the preview
/// values (they feed the response graph), never the pedal's defaults.
struct PedalPreview: View {
    let model: DesignerModel
    private let u: CGFloat = 16

    var body: some View {
        let controls = model.controls
        let color = model.accent
        let shape = RoundedRectangle(cornerRadius: 1.3 * u, style: .continuous)
        ZStack(alignment: .top) {
            Text((model.name.isEmpty ? "Pedal" : model.name).uppercased())
                .font(.system(size: 0.82 * u, weight: .heavy, design: .rounded))
                .tracking(0.1 * u)
                .lineLimit(1).minimumScaleFactor(0.5)
                .foregroundStyle(.black.opacity(0.62))
                .frame(width: 8.4 * u, height: 1.1 * u)
                .padding(.top, 0.85 * u)

            face(controls)
                .frame(width: 8.8 * u, height: 8.2 * u)
                .padding(.top, 2.3 * u)

            Circle().fill(Color(hex: 0xff3b30))
                .overlay(Circle().stroke(.black.opacity(0.35), lineWidth: 0.5))
                .frame(width: 0.62 * u, height: 0.62 * u)
                .padding(.top, 10.9 * u)
            Circle()
                .fill(AngularGradient(colors: [Color(white: 0.55), Color(white: 0.95), Color(white: 0.55), Color(white: 0.95), Color(white: 0.55)], center: .center))
                .frame(width: 3.6 * u, height: 3.6 * u)
                .overlay(Circle().fill(Color(white: 0.85)).frame(width: 1.6 * u, height: 1.6 * u))
                .overlay(Circle().stroke(.black.opacity(0.3), lineWidth: 1))
                .frame(maxHeight: .infinity, alignment: .bottom)
                .padding(.bottom, 1.1 * u)
        }
        .frame(width: 9.4 * u, height: 15.75 * u)
        .background {
            shape
                .fill(LinearGradient(colors: [color.opacity(0.95), color.mix(with: .black, by: 0.35)], startPoint: .top, endPoint: .bottom))
                .overlay(shape.stroke(LinearGradient(colors: [.white.opacity(0.55), .black.opacity(0.3)], startPoint: .top, endPoint: .bottom), lineWidth: 1.2))
                .shadow(color: .black.opacity(0.5), radius: 0.8 * u, y: 0.6 * u)
        }
        .animation(.easeOut(duration: 0.25), value: color)
    }

    @ViewBuilder
    private func face(_ controls: [ControlSpec]) -> some View {
        if controls.isEmpty {
            Text("Add voices to put controls on the face")
                .font(.system(size: 0.7 * u, weight: .semibold)).multilineTextAlignment(.center)
                .foregroundStyle(.black.opacity(0.45))
                .padding(.horizontal, 1.2 * u)
        } else {
            let cols = controls.count <= 1 ? 1 : (controls.count <= 6 ? 2 : 3)
            let rows = (controls.count + cols - 1) / cols
            let cellH = 8.2 * u / CGFloat(rows)
            let cellW = 8.8 * u / CGFloat(cols)
            let knob = min(cellH - 1.05 * u, cellW * 0.72, 3.4 * u)
            VStack(spacing: 0) {
                ForEach(0..<rows, id: \.self) { r in
                    HStack(spacing: 0) {
                        ForEach(Array(controls[(r * cols)..<min((r + 1) * cols, controls.count)])) { c in
                            PreviewControl(model: model, control: c, knob: knob, u: u)
                                .frame(width: cellW, height: cellH)
                        }
                    }
                }
            }
        }
    }
}

extension ControlSpec: Identifiable {}

private struct PreviewControl: View {
    let model: DesignerModel
    let control: ControlSpec
    let knob: CGFloat
    let u: CGFloat

    @State private var startN: Double?

    var body: some View {
        let v = model.previewValue(control)
        VStack(spacing: 0.2 * u) {
            switch control.kind {
            case .knob:
                MiniKnob(n: v, size: knob)
                    .contentShape(Circle().inset(by: -8))
                    .gesture(
                        DragGesture(minimumDistance: 1)
                            .onChanged { g in
                                if startN == nil { startN = v }
                                let delta = (-g.translation.height + g.translation.width) / 160
                                model.setPreview(control.id, min(max((startN ?? v) + delta, 0), 1))
                            }
                            .onEnded { _ in startN = nil }
                    )
                    .onTapGesture(count: 2) { model.values[control.id] = nil }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(control.label)
                    .accessibilityValue(String(format: "%.0f percent", v * 100))
                    .accessibilityAdjustableAction { dir in
                        switch dir {
                        case .increment: model.setPreview(control.id, min(v + 0.05, 1))
                        case .decrement: model.setPreview(control.id, max(v - 0.05, 0))
                        @unknown default: break
                        }
                    }
            case .switch:
                Button { model.setPreview(control.id, v >= 0.5 ? 0 : 1) } label: {
                    Capsule().fill(Color(white: 0.12))
                        .frame(width: knob * 0.9, height: knob * 0.5)
                        .overlay(alignment: v >= 0.5 ? .trailing : .leading) {
                            Circle().fill(v >= 0.5 ? Color(hex: 0xff9f43) : Color(white: 0.55)).padding(3)
                                .frame(width: knob * 0.5, height: knob * 0.5)
                        }
                        .padding(8)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(control.label)
                .accessibilityValue(v >= 0.5 ? "On" : "Off")
            case .selector:
                let options = control.options ?? []
                let i = min(max(Int(v), 0), max(options.count - 1, 0))
                Button { model.setPreview(control.id, Double(options.isEmpty ? 0 : (i + 1) % options.count)) } label: {
                    Text(options.indices.contains(i) ? options[i] : "-")
                        .font(.system(size: 0.66 * u, weight: .bold)).lineLimit(1).minimumScaleFactor(0.6)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 0.5 * u)
                        .frame(width: max(knob * 1.25, 3 * u), height: knob * 0.55)
                        .background(Capsule().fill(Color(white: 0.12)))
                        .padding(6)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(control.label)
                .accessibilityValue(options.indices.contains(i) ? options[i] : "")
            }
            Text(control.label.uppercased())
                .font(.system(size: 0.62 * u, weight: .bold))
                .lineLimit(1).minimumScaleFactor(0.6)
                .foregroundStyle(.black.opacity(0.62))
                .padding(.horizontal, 2)
        }
    }
}

// MARK: - right panel

private struct DesignerRightPanel: View {
    let store: Store
    let model: DesignerModel

    var body: some View {
        @Bindable var model = model
        let problems = model.problems
        return VStack(spacing: 0) {
            Picker("Section", selection: $model.tab) {
                Text("Voices").tag(DesignerTab.voices)
                Text("Inside").tag(DesignerTab.inside)
            }
            .pickerStyle(.segmented)
            .frame(width: 260)
            .padding(.top, 16).padding(.bottom, 8)

            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if !problems.isEmpty { ProblemsCard(problems: problems) }
                    if model.tab == .voices {
                        VoicesTab(store: store, model: model)
                    } else {
                        InsideTab(store: store, model: model)
                    }
                }
                .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, 28)
            }
            .scrollDismissesKeyboard(.interactively)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
    }
}

/// What the core would refuse on Save; shown before it gets there.
private struct ProblemsCard: View {
    let problems: [String]
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(problems.enumerated()), id: \.offset) { _, text in
                Label(text, systemImage: "exclamationmark.triangle.fill")
                    .font(.subheadline)
                    .foregroundStyle(Color(hex: 0xffb454))
                    .labelStyle(ProblemLabelStyle())
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(Color(hex: 0xffb454).opacity(0.12)))
        .accessibilityElement(children: .combine)
    }
}

private struct ProblemLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            configuration.icon.font(.caption)
            configuration.title.fixedSize(horizontal: false, vertical: true)
        }
    }
}
