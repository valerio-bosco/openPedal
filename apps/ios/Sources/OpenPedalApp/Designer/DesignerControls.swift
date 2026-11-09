import SwiftUI

/// A number with minus / plus steppers and a field for typing an exact value.
struct NumberEditor: View {
    @Binding var value: Double
    /// Allowed values and the size of one step.
    let range: ClosedRange<Double>
    let step: Double
    var unit = ""

    @State private var text = ""
    @FocusState private var focused: Bool

    init(value: Binding<Double>, range: ClosedRange<Double>, step: Double, unit: String = "") {
        _value = value
        self.range = range
        self.step = step > 0 ? step : max((range.upperBound - range.lowerBound) / 100, 0.001)
        self.unit = unit
    }

    /// Editor for a numeric block parameter, over its own range.
    init(value: Binding<Double>, param p: ParamDesc) {
        self.init(value: value, range: p.min...p.max, step: p.step, unit: p.unit)
    }

    var body: some View {
        HStack(spacing: 4) {
            stepper("minus", -1)
            TextField("", text: $text)
                .keyboardType(.numbersAndPunctuation)
                .multilineTextAlignment(.center)
                .font(.system(size: 15, weight: .semibold).monospacedDigit())
                .focused($focused)
                .frame(width: 78, height: 34)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.white.opacity(0.1)))
                .onSubmit(commit)
            stepper("plus", 1)
            if !unit.isEmpty {
                Text(unit.hasPrefix(":") ? String(unit.dropFirst()) : unit)
                    .font(.caption).foregroundStyle(.secondary).frame(minWidth: 22, alignment: .leading)
            }
        }
        .onAppear { text = Self.plain(value) }
        .onChange(of: value) { _, v in if !focused { text = Self.plain(v) } }
        .onChange(of: focused) { _, now in if !now { commit() } }
    }

    private func stepper(_ icon: String, _ dir: Double) -> some View {
        Button {
            let next = ((value + dir * step) / step).rounded() * step
            value = min(max(next, range.lowerBound), range.upperBound)
            text = Self.plain(value)
        } label: {
            Image(systemName: icon).font(.system(size: 13, weight: .bold)).frame(width: 34, height: 34).contentShape(Circle())
        }
        .buttonStyle(.glass)
        .buttonBorderShape(.circle)
        .accessibilityLabel(dir < 0 ? "Decrease" : "Increase")
    }

    private func commit() {
        let parsed = Double(text.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: "."))
        if let parsed, parsed.isFinite { value = min(max(parsed, range.lowerBound), range.upperBound) }
        text = Self.plain(value)
    }

    static func plain(_ v: Double) -> String {
        v.formatted(.number.precision(.fractionLength(0...3)).grouping(.never))
    }
}

/// Two-handle slider over a parameter's range (log scale when the parameter is logarithmic).
struct RangeSlider: View {
    @Binding var lo: Double
    @Binding var hi: Double
    let param: ParamDesc
    let color: Color

    private let knob: CGFloat = 28

    var body: some View {
        GeometryReader { geo in
            let w = max(geo.size.width - knob, 1)
            let a = normalized(lo, param), b = normalized(hi, param)
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.14)).frame(height: 6).padding(.horizontal, knob / 2)
                Capsule().fill(color.opacity(0.85))
                    .frame(width: max(w * (b - a), 0), height: 6)
                    .offset(x: knob / 2 + w * a)
                handle.offset(x: w * a)
                    .gesture(drag(w) { n in lo = min(value(forNormalized: n, param), hi) })
                    .accessibilityLabel("From")
                    .accessibilityValue(formatValue(lo, param))
                handle.offset(x: w * b)
                    .gesture(drag(w) { n in hi = max(value(forNormalized: n, param), lo) })
                    .accessibilityLabel("To")
                    .accessibilityValue(formatValue(hi, param))
            }
            .coordinateSpace(name: "range")
            .frame(height: geo.size.height)
        }
        .frame(height: 34)
    }

    private var handle: some View {
        Circle().fill(.white)
            .frame(width: knob, height: knob)
            .shadow(color: .black.opacity(0.35), radius: 3, y: 1)
            .contentShape(Circle().inset(by: -6))
    }

    private func drag(_ w: CGFloat, _ set: @escaping (Double) -> Void) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .named("range"))
            .onChanged { g in set(min(max(Double((g.location.x - knob / 2) / w), 0), 1)) }
    }
}

/// Small drawing of a knob curve: position (x) against parameter value (y), both 0...1.
struct CurveThumb: View {
    let map: MapKind
    let lo: Double
    let hi: Double

    var body: some View {
        Canvas { ctx, size in
            var p = Path()
            let span = hi - lo
            for i in 0...24 {
                let n = Double(i) / 24
                let v = DesignerModel.apply(map, n: n, lo, hi)
                let y = span == 0 ? 0.5 : (v - lo) / span
                let pt = CGPoint(x: 3 + (size.width - 6) * n, y: size.height - 3 - (size.height - 6) * y)
                if i == 0 { p.move(to: pt) } else { p.addLine(to: pt) }
            }
            ctx.stroke(p, with: .foreground, style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
        }
        .frame(width: 34, height: 26)
    }
}
