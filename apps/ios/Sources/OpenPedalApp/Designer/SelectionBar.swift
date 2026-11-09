import SwiftUI

/// Bottom bar of the board's selection mode: how many pedals are marked and "Merge into a pedal".
struct SelectionBar: View {
    let store: Store

    var body: some View {
        let n = store.selection.count
        let ok = store.canMergeSelection
        GlassEffectContainer(spacing: 14) {
            HStack(spacing: 16) {
                Button { store.endSelecting() } label: {
                    Text("Cancel").font(.body.weight(.semibold)).padding(.horizontal, 6)
                }
                .buttonStyle(.glass)
                .controlSize(.large)

                VStack(alignment: .leading, spacing: 2) {
                    Text(n == 0 ? "Tap the pedals to merge" : "\(n) pedal\(n == 1 ? "" : "s") selected")
                        .font(.headline)
                    if n > 0 && !ok {
                        Text("Choose pedals that are next to each other")
                            .font(.caption).foregroundStyle(Color(hex: 0xffb454))
                    } else {
                        Text("The pedals become one pedal with its own controls")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Button { store.openMergeDesigner() } label: {
                    Label("Merge into a pedal", systemImage: "square.stack.3d.up")
                        .font(.body.weight(.semibold))
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .disabled(!ok)
            }
            .padding(.horizontal, 20).padding(.vertical, 14)
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 32, style: .continuous))
        }
        .frame(maxWidth: 760)
    }
}
