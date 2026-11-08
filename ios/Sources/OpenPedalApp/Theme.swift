import SwiftUI

extension Color {
    init(hex: UInt32) {
        self.init(red: Double((hex >> 16) & 0xff) / 255, green: Double((hex >> 8) & 0xff) / 255, blue: Double(hex & 0xff) / 255)
    }
}

/// Enamel colour of a pedal, per node type (as in the mockup), falling back to its category.
func enamel(for type: String, category: String?) -> Color {
    let byType: [String: UInt32] = [
        "gate": 0x8e98a6, "filter": 0x3fb6d3, "octave": 0x9b6cf0, "fuzz": 0xff7a2f, "tonestack": 0xe2c25a,
        "drive": 0xe8423a, "cab": 0x7d8a63, "reverb": 0x4f86f7, "tremolo": 0x2fbf71, "delay": 0x1cb5a3,
        "chorus": 0x5ec8f2, "compressor": 0xd46fb4, "gain": 0xb0b4bb,
    ]
    if let c = byType[type] { return Color(hex: c) }
    let byCategory: [String: UInt32] = [
        "io": 0x6b7280, "dynamics": 0x8e98a6, "drive": 0xe8423a, "filter": 0x3fb6d3, "modulation": 0x2fbf71,
        "time": 0x1cb5a3, "amp": 0x7d8a63, "utility": 0xb0b4bb,
    ]
    return Color(hex: byCategory[category ?? ""] ?? 0xff8a3d)
}

/// "12.5 dB", "1.20 kHz", "0.50": number of decimals follows the range, like the mockup.
func formatValue(_ v: Double, _ p: ParamDesc) -> String {
    let unit = p.unit
    if unit == "Hz", abs(v) >= 1000 { return String(format: "%.2f kHz", v / 1000) }
    let range = p.max - p.min
    let decimals: Int
    if p.step >= 1 { decimals = 0 }
    else if p.step > 0 { decimals = p.step >= 0.1 ? 1 : 2 }
    else if range >= 200 { decimals = 0 }
    else if range >= 20 { decimals = 1 }
    else { decimals = 2 }
    let number = String(format: "%.\(decimals)f", v)
    if unit.isEmpty { return number }
    return unit.hasPrefix(":") ? number + unit : number + " " + unit
}

func formatDb(_ v: Double) -> String { v <= -90 ? "-inf" : String(format: "%.0f", v) }
