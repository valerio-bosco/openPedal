import Foundation

// Codable mirror of ui/src/protocol.ts (contract: docs/PLAN.md §5.4 and §10.2).
// Decoding is tolerant: a backend without `protocol` or `bypass` still works.

/// Highest protocol version this app understands.
let supportedProtocolVersion: UInt32 = 1

enum ParamValue: Codable, Equatable {
    case number(Double)
    case string(String)

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let d = try? c.decode(Double.self) { self = .number(d) }
        else { self = .string(try c.decode(String.self)) }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .number(let d): try c.encode(d)
        case .string(let s): try c.encode(s)
        }
    }
}

struct ParamDesc: Decodable, Identifiable, Hashable {
    enum Kind: String { case number, choice, file }
    let id: String
    let label: String
    let kind: Kind
    var min: Double = 0
    var max: Double = 1
    var def: Double = 0
    var unit: String = ""
    var log: Bool = false
    var step: Double = 0
    var options: [String] = []
    var extensions: [String] = []

    private enum CodingKeys: String, CodingKey {
        case id, label, kind, min, max, `default`, unit, log, step, options, extensions
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? id
        let k = try c.decodeIfPresent(String.self, forKey: .kind) ?? "number"
        kind = Kind(rawValue: k) ?? .number
        min = try c.decodeIfPresent(Double.self, forKey: .min) ?? 0
        max = try c.decodeIfPresent(Double.self, forKey: .max) ?? 1
        def = try c.decodeIfPresent(Double.self, forKey: .default) ?? 0
        unit = try c.decodeIfPresent(String.self, forKey: .unit) ?? ""
        log = try c.decodeIfPresent(Bool.self, forKey: .log) ?? false
        step = try c.decodeIfPresent(Double.self, forKey: .step) ?? 0
        options = try c.decodeIfPresent([String].self, forKey: .options) ?? []
        extensions = try c.decodeIfPresent([String].self, forKey: .extensions) ?? []
    }
}

struct CatalogItem: Decodable, Identifiable, Hashable {
    var id: String { kind }
    let kind: String
    let label: String
    let category: String
    let params: [ParamDesc]
    let composite: Bool
    let description: String?

    private enum CodingKeys: String, CodingKey { case kind, label, category, params, composite, description }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? kind
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? "utility"
        params = try c.decodeIfPresent([ParamDesc].self, forKey: .params) ?? []
        composite = try c.decodeIfPresent(Bool.self, forKey: .composite) ?? false
        description = try c.decodeIfPresent(String.self, forKey: .description)
    }
}

struct NodeSpec: Codable, Identifiable, Equatable {
    var id: String
    var type: String
    var pos: [Double]
    var params: [String: ParamValue]
    var bypass: Bool

    init(id: String, type: String, pos: [Double] = [0, 120], params: [String: ParamValue] = [:], bypass: Bool = false) {
        self.id = id; self.type = type; self.pos = pos; self.params = params; self.bypass = bypass
    }

    private enum CodingKeys: String, CodingKey { case id, type, pos, params, bypass }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        type = try c.decode(String.self, forKey: .type)
        pos = try c.decodeIfPresent([Double].self, forKey: .pos) ?? [0, 120]
        params = try c.decodeIfPresent([String: ParamValue].self, forKey: .params) ?? [:]
        bypass = try c.decodeIfPresent(Bool.self, forKey: .bypass) ?? false
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(type, forKey: .type)
        try c.encode(pos, forKey: .pos)
        try c.encode(params, forKey: .params)
        // Omitted when false, like the TOML files: older servers do not know the field.
        if bypass { try c.encode(true, forKey: .bypass) }
    }
}

struct EdgeSpec: Codable, Equatable {
    var from: String
    var to: String
}

struct GraphFile: Codable, Equatable {
    var name: String
    var version: Int
    var node: [NodeSpec]
    var edge: [EdgeSpec]
}

struct AudioInfo: Decodable, Equatable {
    var sampleRate: Double
    var buffer: Int
    var running: Bool

    private enum CodingKeys: String, CodingKey { case sampleRate = "sample_rate", buffer, running }
}

struct StateMsg: Decodable {
    var catalog: [CatalogItem]
    var graph: GraphFile
    var presets: [String]
    var audio: AudioInfo?
    var protocolVersion: UInt32?

    private enum CodingKeys: String, CodingKey { case catalog, graph, presets, audio, protocolVersion = "protocol" }
}

struct MetersMsg: Decodable {
    var inDb: Double
    var outDb: Double
    var dsp: Double
    var xruns: Int

    private enum CodingKeys: String, CodingKey { case inDb = "in_db", outDb = "out_db", dsp, xruns }
}

struct ErrorMsg: Decodable {
    var message: String
    var node: String?
}

enum ServerMsg {
    case state(StateMsg)
    case meters(MetersMsg)
    case error(ErrorMsg)

    private struct Tag: Decodable { let type: String }

    /// Returns nil for message types this app does not know.
    static func decode(_ data: Data) throws -> ServerMsg? {
        let d = JSONDecoder()
        switch try d.decode(Tag.self, from: data).type {
        case "state": return .state(try d.decode(StateMsg.self, from: data))
        case "meters": return .meters(try d.decode(MetersMsg.self, from: data))
        case "error": return .error(try d.decode(ErrorMsg.self, from: data))
        default: return nil
        }
    }
}

enum ClientMsg {
    case hello
    case setParam(node: String, param: String, value: Double)
    case setBypass(node: String, bypass: Bool)
    case setGraph(GraphFile)
    case loadPreset(String)
    case savePreset(name: String, graph: GraphFile)
    case deletePreset(String)

    private struct Envelope: Encodable {
        var type: String
        var node: String?
        var param: String?
        var value: Double?
        var bypass: Bool?
        var name: String?
        var graph: GraphFile?
    }

    func json() -> String? {
        let e: Envelope
        switch self {
        case .hello: e = Envelope(type: "hello")
        case .setParam(let n, let p, let v): e = Envelope(type: "set_param", node: n, param: p, value: v)
        case .setBypass(let n, let b): e = Envelope(type: "set_bypass", node: n, bypass: b)
        case .setGraph(let g): e = Envelope(type: "set_graph", graph: g)
        case .loadPreset(let n): e = Envelope(type: "load_preset", name: n)
        case .savePreset(let n, let g): e = Envelope(type: "save_preset", name: n, graph: g)
        case .deletePreset(let n): e = Envelope(type: "delete_preset", name: n)
        }
        guard let data = try? JSONEncoder().encode(e) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
