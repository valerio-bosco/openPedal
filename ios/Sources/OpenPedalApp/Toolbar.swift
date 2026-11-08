import SwiftUI

/// Floating glass toolbar: undo/redo, preset switcher, meters, Save.
struct ToolbarView: View {
    let store: Store
    @Binding var showAbout: Bool

    @State private var saveAsName = ""
    @State private var showSaveAs = false
    @State private var showDelete = false

    var body: some View {
        GlassEffectContainer(spacing: 14) {
            HStack(spacing: 14) {
                HStack(spacing: 4) {
                    iconButton("arrow.uturn.backward", "Undo", disabled: !store.canUndo) { store.undo() }
                    iconButton("arrow.uturn.forward", "Redo", disabled: !store.canRedo) { store.redo() }
                }
                .padding(.horizontal, 6)
                .glassEffect(.regular, in: .capsule)

                HStack(spacing: 4) {
                    iconButton("chevron.left", "Previous preset", disabled: store.presets.isEmpty) { store.stepPreset(-1) }
                    presetMenu
                    iconButton("chevron.right", "Next preset", disabled: store.presets.isEmpty) { store.stepPreset(1) }
                }
                .padding(.horizontal, 6)
                .glassEffect(.regular, in: .capsule)

                Spacer(minLength: 8)

                MetersView(model: store.meters)
                    .padding(.horizontal, 16).padding(.vertical, 8)
                    .glassEffect(.regular, in: .capsule)

                Button { store.save() } label: {
                    Label("Save", systemImage: "square.and.arrow.down").font(.body.weight(.semibold))
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .disabled(store.currentPreset.isEmpty)

                Button { showAbout = true } label: { Image(systemName: "info.circle").font(.title3).frame(width: 36, height: 36) }
                    .buttonStyle(.glass)
                    .buttonBorderShape(.circle)
                    .accessibilityLabel("About")
            }
        }
        .alert("Save preset as", isPresented: $showSaveAs) {
            TextField("Name", text: $saveAsName)
            Button("Save") { store.saveAs(saveAsName) }
            Button("Cancel", role: .cancel) {}
        }
        .alert("Delete \"\(store.currentPreset)\"?", isPresented: $showDelete) {
            Button("Delete", role: .destructive) { store.deletePreset(store.currentPreset) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The preset file is removed. What is playing now does not change.")
        }
    }

    private var presetMenu: some View {
        Menu {
            ForEach(store.presets, id: \.self) { name in
                Button { store.loadPreset(name) } label: {
                    if name == store.currentPreset { Label(name, systemImage: "checkmark") } else { Text(name) }
                }
            }
            Divider()
            Button("New", systemImage: "plus") { store.newPreset() }
            Button("Save as…", systemImage: "square.and.arrow.down.on.square") {
                saveAsName = store.currentPreset
                showSaveAs = true
            }
            Button("Delete…", systemImage: "trash", role: .destructive) { showDelete = true }
                .disabled(!store.presets.contains(store.currentPreset))
        } label: {
            HStack(spacing: 6) {
                Text(store.currentPreset.isEmpty ? "Preset" : store.currentPreset)
                    .font(.headline).lineLimit(1)
                if store.dirty { Circle().fill(.orange).frame(width: 7, height: 7).accessibilityLabel("Unsaved changes") }
                Image(systemName: "chevron.down").font(.caption.weight(.bold)).foregroundStyle(.secondary)
            }
            .frame(minWidth: 130)
            .padding(.horizontal, 8).padding(.vertical, 10)
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
    }

    private func iconButton(_ name: String, _ label: String, disabled: Bool, _ action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: name).font(.body.weight(.semibold)).frame(width: 40, height: 40).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(disabled ? 0.35 : 1)
        .disabled(disabled)
        .accessibilityLabel(label)
    }
}

struct MetersView: View {
    let model: MeterModel

    var body: some View {
        HStack(spacing: 12) {
            bar("IN", model.inDb)
            bar("OUT", model.outDb)
        }
    }

    private func bar(_ title: String, _ db: Double) -> some View {
        let level = min(max((db + 60) / 60, 0), 1)
        return HStack(spacing: 6) {
            Text(title).font(.system(size: 10, weight: .bold)).foregroundStyle(.secondary)
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.12))
                Capsule()
                    .fill(LinearGradient(colors: [.green, Color(hex: 0xc5f04a), db > -3 ? .red : Color(hex: 0xff9f43)], startPoint: .leading, endPoint: .trailing))
                    .frame(width: 70 * level)
            }
            .frame(width: 70, height: 6)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(title) level")
        .accessibilityValue("\(formatDb(db)) dB")
    }
}
