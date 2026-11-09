import SwiftUI
import Observation

enum DesignerTab: Hashable { case voices, inside }

/// Enamel swatches offered by the designer (the mockup's palette).
let designerColors: [String] = ["#d9573b", "#e8423a", "#9b6cf0", "#3fb6d3", "#2fbf71", "#e2c25a", "#4f86f7", "#8e98a6"]

/// Everything that makes a draft different from another one; drives the debounced analysis and the discard check.
struct DraftKey: Hashable {
    var name: String
    var category: Category
    var color: String
    var nodes: [NodeSpec]
    var controls: [ControlSpec]
}

/// What the response graph depends on (not the name or colour).
struct AnalysisKey: Hashable {
    var nodes: [NodeSpec]
    var controls: [ControlSpec]
    var values: [String: Double]
}

/// State of one pedal designer. Kept apart from the board's `NodeModel`s: the board is not touched
/// until Save. The inner chain is linear (`pedal_in` -> nodes -> `pedal_out`); a saved pedal with
/// parallel routing (made on the desktop) keeps its edges and only its controls can be edited here.
@MainActor @Observable
final class DesignerModel {
    @ObservationIgnored let store: Store
    /// Id of the saved pedal being edited; nil for a new one.
    let editID: String?
    var name: String
    var category: Category
    var color: String
    var descriptionText: String
    var nodes: [NodeSpec]
    var controls: [ControlSpec]
    /// The original edges when the inner graph is not a simple chain.
    let parallelEdges: [EdgeSpec]?
    /// Preview position of each control: knob 0..1, switch 0/1, selector option index. Missing = default.
    var values: [String: Double] = [:]
    var selectedControl: String?
    var tab: DesignerTab = .voices
    var expandedNode: String?
    @ObservationIgnored private(set) var initialKey: DraftKey?

    var linear: Bool { parallelEdges == nil }
    var isNew: Bool { editID == nil }

    // MARK: creation

    /// A new pedal made of board pedals (they keep their current settings).
    init(store: Store, merging specs: [NodeSpec]) {
        self.store = store
        editID = nil
        name = "New pedal"
        let first = specs.compactMap { store.item(for: $0)?.category }.first { $0 != .io }
        category = first ?? .utility
        color = designerColors[0]
        descriptionText = ""
        nodes = specs
        controls = []
        parallelEdges = nil
        selectedControl = nil
        initialKey = key
    }

    /// An existing pedal file.
    init(store: Store, editing id: String, file: PedalFile) {
        self.store = store
        editID = id
        name = file.name
        category = Category(rawValue: file.category) ?? .utility
        color = file.color ?? designerColors[0]
        descriptionText = file.description
        controls = file.control
        let all = [NodeSpec(id: OpenPedalProtocol.pedalIn, type: OpenPedalProtocol.input)] + file.node
            + [NodeSpec(id: OpenPedalProtocol.pedalOut, type: OpenPedalProtocol.output)]
        let chain = Chain.derive(GraphFile(name: "", version: 2, node: all, edge: file.edge))
        if chain.linear {
            nodes = Array(chain.nodes.dropFirst().dropLast())
            parallelEdges = nil
        } else {
            nodes = file.node
            parallelEdges = file.edge
        }
        selectedControl = file.control.first?.id
        initialKey = key
    }

    var key: DraftKey { DraftKey(name: name, category: category, color: color, nodes: nodes, controls: controls) }
    var hasChanges: Bool { key != initialKey }

    var analysisKey: AnalysisKey { AnalysisKey(nodes: nodes, controls: controls, values: analysisValues) }

    /// Value of every control for `analyze_pedal`.
    var analysisValues: [String: Double] {
        Dictionary(controls.map { ($0.id, previewValue($0)) }, uniquingKeysWith: { a, _ in a })
    }

    // MARK: the file

    /// The pedal as `save_pedal` and `analyze_pedal` take it.
    func pedalFile() -> PedalFile {
        var placed = nodes
        for i in placed.indices { placed[i].pos = [Double(i) * 220, 0] }
        let edges: [EdgeSpec]
        if let parallelEdges {
            edges = parallelEdges
        } else {
            let ids = [OpenPedalProtocol.pedalIn] + nodes.map(\.id) + [OpenPedalProtocol.pedalOut]
            edges = zip(ids, ids.dropFirst()).map { EdgeSpec(from: $0, to: $1) }
        }
        return PedalFile(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            version: 2, category: category.rawValue, description: descriptionText,
            node: placed, edge: edges, control: controls.map(Self.clean), color: color)
    }

    /// Keeps only the route fields that belong to the control's kind.
    private static func clean(_ c: ControlSpec) -> ControlSpec {
        var c = c
        c.label = c.label.trimmingCharacters(in: .whitespacesAndNewlines)
        c.route = c.route.map { r in
            switch c.kind {
            case .knob:
                return RouteSpec(target: r.target, map: r.map == .linear ? nil : r.map, max: r.max, min: r.min, mode: nil)
            case .switch:
                return RouteSpec(target: r.target, mode: r.mode == .add ? .add : nil, off: r.off, on: r.on)
            case .selector:
                return RouteSpec(target: r.target, values: r.values)
            }
        }
        if c.kind != .selector { c.options = nil }
        return c
    }

    /// File id for a pedal that is saved for the first time: the name as a slug, unique among saved pedals.
    func newPedalID() -> String? {
        if let editID { return editID }
        var slug = ""
        for ch in name.lowercased() {
            if ch.isASCII, ch.isLetter || ch.isNumber { slug.append(ch) }
            else if (ch == " " || ch == "-" || ch == "_"), !slug.isEmpty, slug.last != "-" { slug.append("-") }
        }
        slug = String(slug.prefix(48)).trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        guard !slug.isEmpty else { return nil }
        var id = slug, n = 2
        while store.pedals[id] != nil { id = "\(slug)-\(n)"; n += 1 }
        return id
    }

    // MARK: lookups

    func control(_ id: String) -> ControlSpec? { controls.first { $0.id == id } }

    func items(_ node: NodeSpec) -> CatalogItem? { store.item(for: node) }

    func nodeLabel(_ id: String) -> String {
        nodes.first { $0.id == id }.flatMap { store.item(for: $0)?.label } ?? id
    }

    func nodeColor(_ node: NodeSpec) -> Color { enamel(for: node.type, item: store.item(for: node)) }

    var accent: Color { Color(css: color) ?? Color(hex: 0xd9573b) }

    /// "node.param" -> (inner node, descriptor).
    func param(_ target: String) -> (node: NodeSpec, desc: ParamDesc)? {
        let parts = target.split(separator: ".", maxSplits: 1).map(String.init)
        guard parts.count == 2, let node = nodes.first(where: { $0.id == parts[0] }),
              let desc = store.item(for: node)?.params.first(where: { $0.id == parts[1] }) else { return nil }
        return (node, desc)
    }

    // MARK: preview values

    func defaultValue(_ c: ControlSpec) -> Double {
        switch c.default {
        case .boolean(let b): return b ? 1 : 0
        case .number(let v): return c.kind == .switch ? (v != 0 ? 1 : 0) : v
        case nil: return c.kind == .knob ? 0.5 : 0
        }
    }

    func previewValue(_ c: ControlSpec) -> Double { values[c.id] ?? defaultValue(c) }

    func setPreview(_ id: String, _ v: Double) { values[id] = v }

    // MARK: editing controls ("voices")

    func update(_ id: String, _ change: (inout ControlSpec) -> Void) {
        guard let i = controls.firstIndex(where: { $0.id == id }) else { return }
        change(&controls[i])
    }

    func addControl() {
        var n = controls.count + 1
        while controls.contains(where: { $0.id == "v\(n)" }) { n += 1 }
        let c = ControlSpec(id: "v\(n)", label: "Voice \(n)", kind: .knob, route: [], default: .number(0.5), options: nil)
        controls.append(c)
        selectedControl = c.id
    }

    func removeControl(_ id: String) {
        controls.removeAll { $0.id == id }
        values[id] = nil
        if selectedControl == id { selectedControl = controls.first?.id }
    }

    func setKind(_ id: String, _ kind: ControlType) {
        update(id) { c in
            guard c.kind != kind else { return }
            let targets = c.route.map(\.target)
            c.kind = kind
            switch kind {
            case .knob: c.default = .number(0.5); c.options = nil
            case .switch: c.default = .boolean(false); c.options = nil
            case .selector: c.default = .number(0); c.options = ["Position 1", "Position 2", "Position 3"]
            }
            c.route = targets.compactMap { makeRoute($0, kind, optionCount: c.options?.count ?? 0) }
        }
        values[id] = nil
    }

    func addRoute(_ id: String, target: String) {
        update(id) { c in
            guard !c.route.contains(where: { $0.target == target }),
                  let r = makeRoute(target, c.kind, optionCount: c.options?.count ?? 0) else { return }
            c.route.append(r)
        }
    }

    func updateRoute(_ id: String, _ target: String, _ change: (inout RouteSpec) -> Void) {
        update(id) { c in
            guard let i = c.route.firstIndex(where: { $0.target == target }) else { return }
            change(&c.route[i])
        }
    }

    func removeRoute(_ id: String, _ target: String) {
        update(id) { $0.route.removeAll { $0.target == target } }
    }

    func addOption(_ id: String) {
        update(id) { c in
            let n = (c.options?.count ?? 0) + 1
            c.options = (c.options ?? []) + ["Position \(n)"]
            c.route = c.route.map { r in
                var r = r
                r.values = (r.values ?? []) + [nextValue(after: r.values?.last, target: r.target)]
                return r
            }
        }
    }

    func removeOption(_ id: String, at index: Int) {
        update(id) { c in
            guard var o = c.options, o.count > 2, o.indices.contains(index) else { return }
            o.remove(at: index)
            c.options = o
            c.route = c.route.map { r in
                var r = r
                if var v = r.values, v.indices.contains(index) { v.remove(at: index); r.values = v }
                return r
            }
            if case .number(let d) = c.default, Int(d) >= o.count { c.default = .number(Double(o.count - 1)) }
        }
        values[id] = nil
    }

    /// A first route for `target` that fits the control kind, from the parameter's range or options.
    private func makeRoute(_ target: String, _ kind: ControlType, optionCount: Int) -> RouteSpec? {
        guard let p = param(target)?.desc, p.tag != .file else { return nil }
        switch kind {
        case .knob:
            guard p.tag == .number else { return nil }
            return RouteSpec(target: target, map: p.log && p.min > 0 ? .log : nil, max: p.max, min: p.min)
        case .switch:
            if p.tag == .choice {
                let o = p.options
                guard !o.isEmpty else { return nil }
                let d = min(max(Int(p.def), 0), o.count - 1)
                return RouteSpec(target: target, off: .string(o[d]), on: .string(o[(d + 1) % o.count]))
            }
            let on = p.def == p.max ? p.min : p.max
            return RouteSpec(target: target, off: .number(p.def), on: .number(on))
        case .selector:
            let n = max(optionCount, 1)
            let values: [ParamValue] = (0..<n).map { i in
                if p.tag == .choice { return .string(p.options[min(i, p.options.count - 1)]) }
                return .number(n == 1 ? p.def : p.min + (p.max - p.min) * Double(i) / Double(n - 1))
            }
            if p.tag == .choice, p.options.isEmpty { return nil }
            return RouteSpec(target: target, values: values)
        }
    }

    private func nextValue(after v: ParamValue?, target: String) -> ParamValue {
        guard let p = param(target)?.desc, p.tag == .choice, !p.options.isEmpty,
              case .string(let s)? = v, let i = p.options.firstIndex(of: s) else { return v ?? .number(0) }
        return .string(p.options[(i + 1) % p.options.count])
    }

    // MARK: editing the inner chain

    func addNode(kind: String) {
        guard linear, let item = store.item(kind: kind) else { return }
        let base = kind.hasPrefix(OpenPedalProtocol.pedalPrefix) ? String(kind.dropFirst(OpenPedalProtocol.pedalPrefix.count)) : kind
        let taken = Set(nodes.map(\.id))
        var id = base, n = 2
        while taken.contains(id) { id = "\(base)\(n)"; n += 1 }
        nodes.append(NodeSpec(id: id, type: item.kind))
        expandedNode = id
    }

    /// Removes an inner block and every route that targeted it.
    func removeNode(_ id: String) {
        guard linear else { return }
        nodes.removeAll { $0.id == id }
        for i in controls.indices { controls[i].route.removeAll { $0.target.hasPrefix(id + ".") } }
        if expandedNode == id { expandedNode = nil }
    }

    func moveNodes(from: IndexSet, to: Int) {
        guard linear else { return }
        nodes.move(fromOffsets: from, toOffset: to)
    }

    func setNodeParam(_ id: String, _ p: ParamDesc, _ value: ParamValue) {
        guard let i = nodes.firstIndex(where: { $0.id == id }) else { return }
        nodes[i].params[p.id] = value
    }

    func nodeNumber(_ node: NodeSpec, _ p: ParamDesc) -> Double { store.numberValue(node, p) }
    func nodeChoice(_ node: NodeSpec, _ p: ParamDesc) -> Int { store.choiceIndex(node, p) }

    // MARK: what a route does

    /// The value a route gives its parameter at the preview position, as text.
    func currentText(_ c: ControlSpec, _ r: RouteSpec) -> String {
        guard let p = param(r.target)?.desc else { return "-" }
        let v = previewValue(c)
        switch c.kind {
        case .knob:
            guard let lo = r.min, let hi = r.max else { return "-" }
            return formatValue(Self.apply(r.map ?? .linear, n: v, lo, hi), p)
        case .switch:
            let raw = v >= 0.5 ? r.on : r.off
            return (r.mode == .add && v >= 0.5 ? "+" : "") + valueText(raw, p)
        case .selector:
            guard let values = r.values, values.indices.contains(Int(v)) else { return "-" }
            return valueText(values[Int(v)], p)
        }
    }

    /// Text of a route value for its parameter (a number with its unit, or an option name).
    func valueText(_ v: ParamValue?, _ p: ParamDesc) -> String {
        guard let v else { return "-" }
        if p.tag == .choice {
            switch v {
            case .string(let s): return s
            case .number(let n): return p.options.indices.contains(Int(n)) ? p.options[Int(n)] : "-"
            }
        }
        return formatValue(Self.number(v) ?? p.def, p)
    }

    /// The map of the core (`Map::apply`) for a knob position `n` in 0..1.
    static func apply(_ map: MapKind, n: Double, _ lo: Double, _ hi: Double) -> Double {
        let n = min(max(n, 0), 1)
        switch map {
        case .linear: return lo + n * (hi - lo)
        case .inverse: return hi + n * (lo - hi)
        case .log: return lo > 0 && hi > 0 ? lo * pow(hi / lo, n) : lo + n * (hi - lo)
        }
    }

    static func number(_ v: ParamValue?) -> Double? {
        switch v {
        case .number(let n)?: return n
        case .string(let s)?: return Double(s)
        case nil: return nil
        }
    }

    // MARK: validation (the same rules as the core: Library::save_pedal / validate_pedal)

    /// Problems with the pedal as a whole (name, file id, colour).
    var identityProblems: [String] {
        var out: [String] = []
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { out.append("Give the pedal a name.") }
        else if newPedalID() == nil { out.append("The name needs at least one letter or digit.") }
        if Color(css: color) == nil { out.append("The colour must look like #d9573b.") }
        return out
    }

    /// Problems with the inner chain and the voices.
    var structureProblems: [String] {
        var out: [String] = []
        if nodes.isEmpty { out.append("Add at least one block inside the pedal.") }
        var seen = Set<String>()
        for c in controls {
            let at = "Voice \"\(c.label.isEmpty ? c.id : c.label)\""
            if c.label.trimmingCharacters(in: .whitespaces).isEmpty { out.append("A voice has no name.") }
            if !seen.insert(c.id).inserted { out.append("\(at): duplicate id \"\(c.id)\".") }
            if c.kind == .selector {
                let o = c.options ?? []
                if o.count < 2 { out.append("\(at): a selector needs at least two positions.") }
                if o.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty }) { out.append("\(at): a position has no name.") }
            }
            for r in c.route { out.append(contentsOf: problems(of: r, in: c, at: at)) }
        }
        return out
    }

    var problems: [String] { identityProblems + structureProblems }

    /// Problems with a single route, for the inline warning under it.
    func problems(of r: RouteSpec, in c: ControlSpec) -> [String] {
        problems(of: r, in: c, at: "")
    }

    private func problems(of r: RouteSpec, in c: ControlSpec, at prefix: String) -> [String] {
        let at = prefix.isEmpty ? "" : prefix + ": "
        guard let (node, p) = param(r.target) else {
            return ["\(at)\(r.target) does not exist any more."]
        }
        let name = "\(nodeLabel(node.id)) \(p.label)"
        let nested = node.type.hasPrefix(OpenPedalProtocol.pedalPrefix)
        var out: [String] = []
        if p.tag == .file { return ["\(at)\(name) is a file and cannot be driven."] }
        if r.mode == .add, nested || p.tag != .number { out.append("\(at)add mode needs a numeric block parameter, and \(name) is not one.") }
        switch c.kind {
        case .knob:
            if p.tag == .choice { return out + ["\(at)\(name) is a choice: drive it with a switch or a selector."] }
            guard let lo = r.min, let hi = r.max, lo.isFinite, hi.isFinite else { return out + ["\(at)\(name) has no range."] }
            if r.map == .log, lo <= 0 || hi <= 0 { out.append("\(at)\(name): a log curve needs a range above zero.") }
        case .switch:
            for (label, v) in [("off", r.off), ("on", r.on)] where !valid(v, p) { out.append("\(at)\(name): the \(label) value is not valid.") }
        case .selector:
            let values = r.values ?? []
            if values.count != (c.options ?? []).count { out.append("\(at)\(name): needs one value per position.") }
            else if values.contains(where: { !valid($0, p) }) { out.append("\(at)\(name): a position has no valid value.") }
        }
        return out
    }

    private func valid(_ v: ParamValue?, _ p: ParamDesc) -> Bool {
        guard let v else { return false }
        if p.tag == .choice {
            switch v {
            case .string(let s): return p.options.contains(s)
            case .number(let n): return n >= 0 && n.rounded() == n && Int(n) < p.options.count
            }
        }
        if case .number(let n) = v { return n.isFinite }
        return false
    }
}
