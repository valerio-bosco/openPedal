import SwiftUI

/// Magnitude response of the pedal: log frequency (20 Hz to 20 kHz) against dB, filled in the pedal colour.
struct ResponseGraph: View {
    let model: ResponseModel
    let color: Color

    private static let fMin = 20.0
    private static let fMax = 20000.0

    var body: some View {
        let freq = model.freq, db = model.db
        Canvas { ctx, size in
            draw(ctx, size, freq, db)
        }
        .drawingGroup()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Frequency response")
        .accessibilityValue(summary(db))
    }

    private func summary(_ db: [Double]) -> String {
        guard let lo = db.min(), let hi = db.max() else { return "Not measured yet" }
        return String(format: "From %.0f to %.0f dB", lo, hi)
    }

    /// dB range of the plot: whole 6 dB steps around the data, at least 24 dB tall, always containing 0 dB.
    private func range(_ db: [Double]) -> (lo: Double, hi: Double) {
        guard let mn = db.min(), let mx = db.max() else { return (-24, 12) }
        var hi = max((mx / 6).rounded(.up) * 6, 6)
        var lo = min((mn / 6).rounded(.down) * 6, -12)
        lo = max(lo, -72)
        if hi - lo < 24 { hi = lo + 24 }
        if hi < mx { hi = (mx / 6).rounded(.up) * 6 }
        return (lo, hi)
    }

    private func draw(_ ctx: GraphicsContext, _ size: CGSize, _ freq: [Double], _ db: [Double]) {
        let left: CGFloat = 30, right: CGFloat = 6, top: CGFloat = 6, bottom: CGFloat = 16
        let plot = CGRect(x: left, y: top, width: max(size.width - left - right, 1), height: max(size.height - top - bottom, 1))
        let (lo, hi) = range(db)
        let lf0 = log10(Self.fMin), lf1 = log10(Self.fMax)
        func x(_ f: Double) -> CGFloat { plot.minX + plot.width * CGFloat((log10(f) - lf0) / (lf1 - lf0)) }
        func y(_ v: Double) -> CGFloat { plot.maxY - plot.height * CGFloat((min(max(v, lo), hi) - lo) / (hi - lo)) }
        let grid = Color.white.opacity(0.1)
        let label = Color.white.opacity(0.45)

        // dB grid: every 12 dB (every 6 when the range is small).
        let step = (hi - lo) <= 36 ? 6.0 : 12.0
        var v = (lo / step).rounded(.up) * step
        while v <= hi {
            var line = Path()
            line.move(to: CGPoint(x: plot.minX, y: y(v)))
            line.addLine(to: CGPoint(x: plot.maxX, y: y(v)))
            ctx.stroke(line, with: .color(v == 0 ? Color.white.opacity(0.3) : grid), lineWidth: v == 0 ? 1 : 0.6)
            ctx.draw(Text(String(format: "%.0f", v)).font(.system(size: 9, weight: .medium).monospacedDigit()).foregroundStyle(label),
                     at: CGPoint(x: plot.minX - 4, y: y(v)), anchor: .trailing)
            v += step
        }
        // Frequency grid.
        for (f, text) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
            var line = Path()
            line.move(to: CGPoint(x: x(f), y: plot.minY))
            line.addLine(to: CGPoint(x: x(f), y: plot.maxY))
            ctx.stroke(line, with: .color(grid), lineWidth: 0.6)
            if ["100", "1k", "10k"].contains(text) {
                ctx.draw(Text(text).font(.system(size: 9, weight: .medium)).foregroundStyle(label),
                         at: CGPoint(x: x(f), y: plot.maxY + 9), anchor: .center)
            }
        }

        guard freq.count > 1, freq.count == db.count else { return }
        var curve = Path()
        for i in freq.indices {
            let p = CGPoint(x: x(freq[i]), y: y(db[i]))
            if i == 0 { curve.move(to: p) } else { curve.addLine(to: p) }
        }
        var area = curve
        area.addLine(to: CGPoint(x: x(freq[freq.count - 1]), y: plot.maxY))
        area.addLine(to: CGPoint(x: x(freq[0]), y: plot.maxY))
        area.closeSubpath()
        ctx.fill(area, with: .linearGradient(Gradient(colors: [color.opacity(0.55), color.opacity(0.05)]),
                                              startPoint: CGPoint(x: 0, y: plot.minY), endPoint: CGPoint(x: 0, y: plot.maxY)))
        ctx.stroke(curve, with: .color(color), style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
    }
}
