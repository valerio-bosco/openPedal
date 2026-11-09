import Foundation

// Hand-written conveniences on top of the generated wire types in Generated/Protocol.swift
// (contract: docs/PLAN.md §5.4 and §10.2). Do not redefine wire types here.

/// Highest protocol version this app understands.
let supportedProtocolVersion = 2

extension ParamDesc: Identifiable {
    enum Tag { case number, choice, file }

    var tag: Tag {
        switch kind {
        case .number: return .number
        case .choice: return .choice
        case .file: return .file
        }
    }

    var min: Double { if case .number(let v, _, _, _, _, _) = kind { return v } else { return 0 } }
    var max: Double { if case .number(_, let v, _, _, _, _) = kind { return v } else { return 1 } }
    var def: Double {
        switch kind {
        case .number(_, _, let d, _, _, _): return d
        case .choice(_, let d): return Double(d)
        case .file: return 0
        }
    }
    var unit: String { if case .number(_, _, _, let v, _, _) = kind { return v } else { return "" } }
    var log: Bool { if case .number(_, _, _, _, let v, _) = kind { return v } else { return false } }
    var step: Double { if case .number(_, _, _, _, _, let v) = kind { return v } else { return 0 } }
    var options: [String] { if case .choice(let o, _) = kind { return o } else { return [] } }
    var extensions: [String] { if case .file(let e) = kind { return e } else { return [] } }
}

extension CatalogItem: Identifiable {
    var id: String { kind }
}

extension NodeSpec: Identifiable {
    /// A node as the editor creates it: laid out later, with no parameters set.
    init(id: String, type: String, pos: [Double] = [0, 120], params: ParamValues = [:]) {
        self.init(id: id, type: type, pos: pos, params: params, bypass: nil)
    }

    /// `bypass` is omitted on the wire when false.
    var isBypassed: Bool {
        get { bypass ?? false }
        set { bypass = newValue ? true : nil }
    }
}

extension ServerMsg {
    static func decode(_ data: Data) throws -> ServerMsg {
        try JSONDecoder().decode(ServerMsg.self, from: data)
    }
}

extension ClientMsg {
    func json() -> String? {
        guard let data = try? JSONEncoder().encode(self) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
