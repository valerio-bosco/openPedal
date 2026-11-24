import SwiftUI

/// Waveform of the mix with a time ruler, the song map under it, and a draggable selection.
struct WaveformView: View {
    let peaks: [Float]
    let duration: Double
    let sections: [TonelabSection]
    @Binding var selection: ClosedRange<Double>?
    var playhead: Double?

    private enum Drag { case new(anchor: Double), start, end }
    @State private var drag: Drag?

    private let rulerH: CGFloat = 18
    private let mapH: CGFloat = 8

    var body: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let h = geo.size.height
            let waveH = h - rulerH - (sections.isEmpty ? 0 : mapH + 2)
            Canvas { ctx, size in
                // Song map strip.
                var y: CGFloat = 0
                if !sections.isEmpty {
                    for s in sections {
                        let r = CGRect(x: s.start / duration * w, y: 0, width: max(1, (s.end - s.start) / duration * w), height: mapH)
                        ctx.fill(Path(roundedRect: r, cornerRadius: 2), with: .color(tlKindColor[s.kind] ?? .gray))
                    }
                    y = mapH + 2
                }
                // Sections tint behind the wave.
                for s in sections where s.kind != "none" {
                    let r = CGRect(x: s.start / duration * w, y: y, width: (s.end - s.start) / duration * w, height: waveH)
                    ctx.fill(Path(r), with: .color((tlKindColor[s.kind] ?? .gray).opacity(0.12)))
                }
                // Peaks.
                let mid = y + waveH / 2
                var p = Path()
                let n = peaks.count
                if n > 0 {
                    let step = w / CGFloat(n)
                    for (i, v) in peaks.enumerated() {
                        let a = CGFloat(v) * waveH / 2
                        p.addRect(CGRect(x: CGFloat(i) * step, y: mid - a, width: max(1, step - 0.3), height: max(1, a * 2)))
                    }
                }
                ctx.fill(p, with: .color(.white.opacity(0.75)))
                // Selection.
                if let sel = selection {
                    let r = CGRect(x: sel.lowerBound / duration * w, y: y, width: (sel.upperBound - sel.lowerBound) / duration * w, height: waveH)
                    ctx.fill(Path(r), with: .color(Color(hex: 0xff8a3d).opacity(0.28)))
                    for x in [r.minX, r.maxX] {
                        ctx.fill(Path(CGRect(x: x - 1.5, y: y, width: 3, height: waveH)), with: .color(Color(hex: 0xff8a3d)))
                    }
                }
                if let t = playhead {
                    ctx.fill(Path(CGRect(x: t / duration * w - 1, y: y, width: 2, height: waveH)), with: .color(.white))
                }
                // Ruler.
                let step = tlTickStep(duration)
                var t = 0.0
                while t <= duration {
                    let x = t / duration * w
                    ctx.fill(Path(CGRect(x: x, y: y + waveH, width: 1, height: 5)), with: .color(.white.opacity(0.5)))
                    ctx.draw(Text(tlClock(t)).font(.system(size: 10)).foregroundStyle(.secondary), at: CGPoint(x: min(max(x, 14), w - 14), y: y + waveH + 12))
                    t += step
                }
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        guard duration > 0, w > 0 else { return }
                        let t = min(max(0, v.location.x / w * duration), duration)
                        if drag == nil {
                            let t0 = min(max(0, v.startLocation.x / w * duration), duration)
                            let grab = 28 / w * duration
                            if let s = selection, abs(t0 - s.lowerBound) < grab, abs(t0 - s.lowerBound) <= abs(t0 - s.upperBound) { drag = .start }
                            else if let s = selection, abs(t0 - s.upperBound) < grab { drag = .end }
                            else { drag = .new(anchor: t0) }
                        }
                        switch drag {
                        case .new(let a): selection = min(a, t)...max(a, t)
                        case .start: if let s = selection { selection = min(t, s.upperBound)...s.upperBound }
                        case .end: if let s = selection { selection = s.lowerBound...max(t, s.lowerBound) }
                        case nil: break
                        }
                    }
                    .onEnded { _ in
                        drag = nil
                        if let s = selection, s.upperBound - s.lowerBound < 0.5 { selection = nil }
                    }
            )
        }
        .accessibilityLabel("Waveform of the song. Drag to select the part.")
    }
}

/// Scrub bar for the A/B player.
struct ScrubBar: View {
    let value: Double
    let total: Double
    let onSeek: (Double) -> Void

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.15))
                Capsule().fill(Color(hex: 0xff8a3d)).frame(width: geo.size.width * (total > 0 ? min(1, value / total) : 0))
            }
            .frame(height: 6)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { v in
                onSeek(min(max(0, v.location.x / geo.size.width), 1) * total)
            })
        }
        .frame(height: 28)
    }
}
