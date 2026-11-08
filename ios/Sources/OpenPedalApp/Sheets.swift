import SwiftUI

/// Bottom sheet listing the catalog; tapping an entry inserts it after the selected pedal.
struct AddPedalSheet: View {
    let store: Store
    @Environment(\.dismiss) private var dismiss

    private var groups: [(String, [CatalogItem])] {
        let items = store.catalog.filter { $0.category != "io" && $0.kind != "input" && $0.kind != "output" }
        let cats = Array(Set(items.map(\.category))).sorted()
        return cats.map { c in (c, items.filter { $0.category == c }) }
    }

    var body: some View {
        NavigationStack {
            List {
                if !store.canEditChain {
                    Label("Parallel routing: edit on the desktop", systemImage: "info.circle").foregroundStyle(.secondary)
                }
                ForEach(groups, id: \.0) { cat, items in
                    Section(cat.capitalized) {
                        ForEach(items) { item in
                            Button {
                                store.addPedal(item)
                                dismiss()
                            } label: {
                                HStack(spacing: 14) {
                                    RoundedRectangle(cornerRadius: 6)
                                        .fill(enamel(for: item.kind, category: item.category))
                                        .frame(width: 18, height: 30)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(item.label).font(.body.weight(.semibold))
                                        if let d = item.description, !d.isEmpty {
                                            Text(d).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                        }
                                    }
                                    Spacer()
                                    if item.composite { Text("custom").font(.caption2).foregroundStyle(.secondary) }
                                }
                            }
                            .disabled(!store.canEditChain)
                        }
                    }
                }
            }
            .navigationTitle("Add pedal")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
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
                        row("Sample rate", String(format: "%.0f Hz", a.sampleRate))
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
