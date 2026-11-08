import SwiftUI

/// Lightweight static stompbox used by the Add pedal browser: body gradient, name plate, a few knobs,
/// LED and footswitch. Flattened into one bitmap so horizontal rows scroll smoothly.
struct PedalThumbnail: View {
    let item: CatalogItem

    private let w: CGFloat = 64
    private let h: CGFloat = 108

    var body: some View {
        let color = enamel(for: item.kind, category: item.category.rawValue)
        let numbers = Array(item.params.filter { $0.tag == .number }.prefix(4))
        let shape = RoundedRectangle(cornerRadius: 9, style: .continuous)

        ZStack(alignment: .top) {
            shape.fill(LinearGradient(colors: [color.opacity(0.95), color.opacity(0.62)], startPoint: .top, endPoint: .bottom))
            shape.strokeBorder(.white.opacity(0.28), lineWidth: 0.8)
            VStack(spacing: 5) {
                Text(item.label.uppercased())
                    .font(.system(size: 6.5, weight: .heavy, design: .rounded))
                    .lineLimit(1).minimumScaleFactor(0.6)
                    .foregroundStyle(.black.opacity(0.7))
                    .padding(.horizontal, 5)
                Circle().fill(Color(red: 1, green: 0.35, blue: 0.3)).frame(width: 5, height: 5)
                knobs(numbers)
                Spacer(minLength: 0)
                Circle()
                    .fill(AngularGradient(colors: [Color(white: 0.55), Color(white: 0.95), Color(white: 0.55), Color(white: 0.95), Color(white: 0.55)], center: .center))
                    .frame(width: 20, height: 20)
                    .overlay(Circle().fill(Color(white: 0.85)).frame(width: 8, height: 8))
                    .overlay(Circle().stroke(.black.opacity(0.3), lineWidth: 0.8))
            }
            .padding(.vertical, 9)
            if item.composite {
                Image(systemName: "square.stack.3d.up.fill")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.white)
                    .padding(4)
                    .background(Circle().fill(.black.opacity(0.55)))
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                    .padding(3)
            }
        }
        .frame(width: w, height: h)
        .shadow(color: .black.opacity(0.3), radius: 3, y: 2)
        .drawingGroup()
    }

    @ViewBuilder private func knobs(_ numbers: [ParamDesc]) -> some View {
        let size: CGFloat = numbers.count > 2 ? 16 : 22
        let rows = numbers.count > 2 ? [Array(numbers.prefix(2)), Array(numbers.dropFirst(2))] : [numbers]
        VStack(spacing: 4) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                HStack(spacing: 6) {
                    ForEach(row) { p in MiniKnob(n: normalized(p.def, p), size: size) }
                }
            }
        }
    }
}
