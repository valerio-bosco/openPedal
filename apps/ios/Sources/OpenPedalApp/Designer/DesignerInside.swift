import SwiftUI

/// "Inside" tab: the blocks of the inner chain (pedal_in to pedal_out), in signal order.
struct InsideTab: View {
    let store: Store
    let model: DesignerModel
    @State private var adding = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !model.linear {
                Label("Parallel routing: edit the structure on the desktop. Voices can still be changed here.", systemImage: "info.circle")
                    .font(.subheadline).foregroundStyle(.secondary)
                    .padding(14).frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(.white.opacity(0.06)))
            }
            if model.nodes.isEmpty {
                Text("No blocks yet. Add the blocks the pedal is made of, in signal order.")
                    .foregroundStyle(.secondary).padding(.horizontal, 4)
            }
            // A List gives drag handles for reordering; it sits inside the panel's scroll view at its full height.
            List {
                ForEach(model.nodes) { node in
                    NodeRow(store: store, model: model, node: node)
                        .listRowBackground(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(.white.opacity(0.06)).padding(.vertical, 3))
                        .listRowSeparator(.hidden)
                        .listRowInsets(EdgeInsets(top: 3, leading: 6, bottom: 3, trailing: 6))
                        .deleteDisabled(true)
                        .moveDisabled(!model.linear)
                }
                .onMove { model.moveNodes(from: $0, to: $1) }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .scrollDisabled(true)
            .environment(\.editMode, .constant(model.linear ? .active : .inactive))
            .frame(height: listHeight)

            if model.linear {
                Button { adding = true } label: {
                    Label("Add block", systemImage: "plus").font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity).frame(height: 48).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(.white.opacity(0.25), style: StrokeStyle(lineWidth: 1.5, dash: [6, 5])))
            }
            Text("Inner blocks keep their own settings; the voices override the parameters they drive. On iPad the inner chain is linear; parallel routing is edited on the desktop.")
                .font(.footnote).foregroundStyle(.tertiary).padding(.horizontal, 4)
        }
        .sheet(isPresented: $adding) { BlockPicker(store: store, model: model) }
    }

    /// Rows are 62 pt, and a block with its settings open is taller.
    private var listHeight: CGFloat {
        model.nodes.reduce(0) { sum, n in
            let open = model.expandedNode == n.id
            let rows = open ? (model.items(n)?.params.filter { $0.tag != .file }.count ?? 0) : 0
            return sum + 68 + CGFloat(rows) * 52 + (open ? 12 : 0)
        }
    }
}

private struct NodeRow: View {
    let store: Store
    let model: DesignerModel
    let node: NodeSpec

    var body: some View {
        let item = model.items(node)
        let open = model.expandedNode == node.id
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Circle().fill(model.nodeColor(node)).frame(width: 14, height: 14)
                VStack(alignment: .leading, spacing: 1) {
                    Text(item?.label ?? node.type).font(.body.weight(.semibold))
                    Text(node.id + (item?.composite == true ? " · pedal" : "")).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Spacer()
                Button { withAnimation(.smooth) { model.expandedNode = open ? nil : node.id } } label: {
                    Image(systemName: "chevron.right").font(.footnote.weight(.semibold))
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .frame(width: 36, height: 36).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(open ? "Hide settings" : "Show settings")
                if model.linear {
                    Button(role: .destructive) { withAnimation(.smooth) { model.removeNode(node.id) } } label: {
                        Image(systemName: "trash").font(.body).frame(width: 36, height: 36).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove \(item?.label ?? node.type)")
                }
            }
            .frame(height: 56)
            if open, let item {
                VStack(spacing: 6) {
                    ForEach(item.params.filter { $0.tag != .file }) { p in
                        InnerParamRow(model: model, node: node, p: p, tint: model.accent)
                    }
                }
                .padding(.bottom, 10)
            }
        }
        .padding(.horizontal, 8)
    }
}

/// One setting of an inner block: a slider for a number, a menu for a choice.
private struct InnerParamRow: View {
    let model: DesignerModel
    let node: NodeSpec
    let p: ParamDesc
    let tint: Color

    var body: some View {
        // Read the live node: `node` is the row's snapshot.
        let live = model.nodes.first { $0.id == node.id } ?? node
        HStack(spacing: 12) {
            Text(p.label).font(.subheadline).foregroundStyle(.secondary).frame(width: 130, alignment: .leading).lineLimit(1)
            if p.tag == .number {
                let v = model.nodeNumber(live, p)
                Slider(value: Binding(
                    get: { normalized(model.nodeNumber(live, p), p) },
                    set: { model.setNodeParam(node.id, p, .number(value(forNormalized: $0, p))) }), in: 0...1)
                    .tint(tint)
                Text(formatValue(v, p)).font(.subheadline.monospacedDigit()).frame(width: 84, alignment: .trailing)
            } else {
                Picker(p.label, selection: Binding(
                    get: { model.nodeChoice(live, p) },
                    set: { i in if p.options.indices.contains(i) { model.setNodeParam(node.id, p, .string(p.options[i])) } })) {
                    ForEach(Array(p.options.enumerated()), id: \.offset) { i, o in Text(o.capitalized).tag(i) }
                }
                .pickerStyle(.menu)
                Spacer()
            }
        }
        .frame(height: 46)
    }
}

/// Catalog of blocks to add inside the pedal (everything but the jacks and the pedal itself).
struct BlockPicker: View {
    let store: Store
    let model: DesignerModel
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    private var groups: [(String, [CatalogItem])] {
        let own = model.editID.map { OpenPedalProtocol.pedalPrefix + $0 }
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        let items = store.catalog.filter { c in
            c.category != .io && c.kind != own
                && (q.isEmpty || c.label.lowercased().contains(q) || c.kind.lowercased().contains(q))
        }
        let names = Array(Set(items.map { $0.composite ? "custom" : $0.category.rawValue })).sorted()
        return names.map { n in (n, items.filter { ($0.composite ? "custom" : $0.category.rawValue) == n }) }
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(groups, id: \.0) { name, items in
                    Section(name.capitalized) {
                        ForEach(items) { item in
                            Button {
                                model.addNode(kind: item.kind)
                                dismiss()
                            } label: {
                                HStack(spacing: 12) {
                                    Circle().fill(enamel(for: item.kind, item: item)).frame(width: 14, height: 14)
                                    VStack(alignment: .leading, spacing: 1) {
                                        Text(item.label)
                                        if let d = item.description, !d.isEmpty {
                                            Text(d).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("Add block")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "Search blocks")
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}
