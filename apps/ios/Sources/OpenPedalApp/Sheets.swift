import SwiftUI

/// Bottom sheet for browsing the catalog: one horizontal row of pedal tiles per category.
/// Tapping a tile inserts the pedal after the selected one.
struct AddPedalSheet: View {
    let store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""

    private static let order = ["dynamics", "drive", "amp", "filter", "modulation", "time", "utility"]

    private func section(of item: CatalogItem) -> String { item.composite ? "custom" : item.category.rawValue }

    private var groups: [(String, [CatalogItem])] {
        let items = store.catalog.filter { $0.category != .io && $0.kind != "input" && $0.kind != "output" }
        let present = Set(items.map { section(of: $0) })
        let known = Self.order.filter { present.contains($0) }
        let rest = present.subtracting(Self.order).subtracting(["custom"]).sorted()
        let cats = known + rest + (present.contains("custom") ? ["custom"] : [])
        return cats.map { c in (c, items.filter { section(of: $0) == c }) }
    }

    private var searching: Bool { !query.trimmingCharacters(in: .whitespaces).isEmpty }

    private var matches: [CatalogItem] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        return groups.flatMap { $0.1 }.filter {
            $0.label.lowercased().contains(q) || $0.kind.lowercased().contains(q)
                || $0.category.rawValue.lowercased().contains(q) || ($0.description ?? "").lowercased().contains(q)
        }
    }

    private func add(_ item: CatalogItem) {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        store.addPedal(item)
        dismiss()
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 22) {
                    if !store.canEditChain {
                        Label("Parallel routing: edit on the desktop", systemImage: "info.circle")
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 20)
                    }
                    if !searching {
                        ForEach(groups, id: \.0) { cat, items in
                            VStack(alignment: .leading, spacing: 8) {
                                HStack(alignment: .firstTextBaseline) {
                                    Text(cat.capitalized).font(.title3.weight(.semibold))
                                    Text("\(items.count)").font(.subheadline).foregroundStyle(.secondary)
                                }
                                .padding(.horizontal, 20)
                                ScrollView(.horizontal, showsIndicators: false) {
                                    LazyHStack(alignment: .top, spacing: 14) {
                                        ForEach(items) { item in tile(item) }
                                    }
                                    .scrollTargetLayout()
                                }
                                .contentMargins(.horizontal, 20, for: .scrollContent)
                                .scrollTargetBehavior(.viewAligned)
                            }
                            .opacity(store.canEditChain ? 1 : 0.5)
                        }
                    } else if matches.isEmpty {
                        ContentUnavailableView.search(text: query).frame(maxWidth: .infinity)
                    } else {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 104), spacing: 14, alignment: .top)], alignment: .leading, spacing: 18) {
                            ForEach(matches) { item in tile(item) }
                        }
                        .padding(.horizontal, 20)
                        .opacity(store.canEditChain ? 1 : 0.5)
                    }
                }
                .padding(.vertical, 12)
            }
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("Add pedal")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $query, prompt: "Search pedals")
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private func tile(_ item: CatalogItem) -> some View {
        Button { add(item) } label: {
            VStack(spacing: 6) {
                PedalThumbnail(item: item)
                Text(item.label)
                    .font(.footnote.weight(.semibold))
                    .lineLimit(1)
                if let d = item.description, !d.isEmpty {
                    Text(d).font(.caption2).foregroundStyle(.secondary)
                        .lineLimit(2).multilineTextAlignment(.center)
                }
            }
            .frame(width: 104, alignment: .top)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!store.canEditChain)
        .accessibilityLabel("\(item.label), \(section(of: item))")
    }
}

struct AboutSheet: View {
    let store: Store
    @Environment(\.dismiss) private var dismiss

    private func info(_ key: String) -> String {
        Bundle.main.object(forInfoDictionaryKey: key) as? String ?? "?"
    }

    var body: some View {
        NavigationStack {
            List {
                Section("App") {
                    row("Version", info("CFBundleShortVersionString"))
                    row("Build", info("CFBundleVersion"))
                }
                Section("Core") {
                    row("Protocol", store.serverProtocol.map { "v\($0)" } ?? "not reported (pre-v1 core)")
                    row("App supports", "v\(supportedProtocolVersion)")
                    if let a = store.audio {
                        row("Audio", a.running ? "running" : "stopped")
                        row("Sample rate", String(format: "%.0f Hz", Double(a.sampleRate)))
                        row("Buffer", "\(a.buffer) frames")
                    }
                    row("Server", store.connected ? "connected" : "disconnected")
                }
            }
            .navigationTitle("openPedal")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.medium])
    }

    private func row(_ k: String, _ v: String) -> some View {
        LabeledContent(k) { Text(v).foregroundStyle(.secondary) }
    }
}
