import Foundation
import Observation

/// Input/output levels, kept apart from `Store` so 30 Hz updates only redraw the meters.
@MainActor @Observable
final class MeterModel {
    var inDb: Double = -90
    var outDb: Double = -90
    var dsp: Double = 0
    var xruns: Int = 0
}

/// One node's live values. Views of a pedal observe only its own model, so turning a knob redraws
/// that pedal and that control, not the whole board.
@MainActor @Observable
final class NodeModel: Identifiable {
    let id: String
    let type: String
    var spec: NodeSpec
    init(_ spec: NodeSpec) { id = spec.id; type = spec.type; self.spec = spec }
}

@MainActor @Observable
final class Store {
    private(set) var connected = false
    private(set) var catalog: [CatalogItem] = []
    private(set) var catalogByKind: [String: CatalogItem] = [:]
    /// Structure (order, edges, name). Node values live in `models` and are folded back by `snapshot()`.
    private(set) var graph = GraphFile(name: "", version: 2, node: [], edge: [])
    private(set) var presets: [String] = []
    private(set) var audio: AudioInfo?
    private(set) var serverProtocol: Int?
    private(set) var gotState = false
    /// From the core (`state`): shared history and unsaved changes.
    private(set) var dirty = false
    private(set) var canUndo = false
    private(set) var canRedo = false
    private(set) var chain = Chain(nodes: [], linear: true)
    private(set) var models: [String: NodeModel] = [:]
    var selectedID: String?
    var banner: String?

    let meters = MeterModel()

    /// The server speaks a newer contract than this app knows.
    var protocolTooNew: Bool { (serverProtocol ?? 0) > supportedProtocolVersion }
    var canEditChain: Bool { chain.linear }
    /// At least one node besides the input and output jacks.
    var hasPedals: Bool { chain.nodes.contains { $0.type != "input" && $0.type != "output" } }

    private let client: WSClient
    private var lastLocalEdit: [String: Date] = [:]
    private var bannerTask: Task<Void, Never>?
    private var pending: [String: ClientMsg] = [:]
    private var flushScheduled = false
    private var lastFlush = Date.distantPast

    init(port: UInt16) {
        client = WSClient(port: port)
        client.onStatus = { [weak self] up in
            guard let self else { return }
            self.connected = up
        }
        client.onMessage = { [weak self] msg in self?.handle(msg) }
        client.start()
    }

    // MARK: lookups

    func item(for node: NodeSpec) -> CatalogItem? { catalogByKind[node.type] }
    func item(kind: String) -> CatalogItem? { catalogByKind[kind] }
    var selectedModel: NodeModel? { selectedID.flatMap { models[$0] } }
    /// Type of the selected node, from the structure only (does not observe its values).
    var selectedType: String? { chain.nodes.first { $0.id == selectedID }?.type }

    /// The graph with the current node values, for saving, undo and structural edits.
    func snapshot() -> GraphFile {
        var g = graph
        g.node = g.node.map { models[$0.id]?.spec ?? $0 }
        return g
    }

    /// Chain order with the current node values.
    private func currentOrder() -> [NodeSpec] { chain.nodes.map { models[$0.id]?.spec ?? $0 } }

    func numberValue(_ node: NodeSpec, _ p: ParamDesc) -> Double {
        switch node.params[p.id] {
        case .number(let v): return v
        case .string(let s): return Double(s) ?? p.def
        case nil: return p.def
        }
    }

    func choiceIndex(_ node: NodeSpec, _ p: ParamDesc) -> Int {
        switch node.params[p.id] {
        case .string(let s): return p.options.firstIndex(of: s) ?? Int(p.def)
        case .number(let v): return Int(v)
        case nil: return Int(p.def)
        }
    }

    // MARK: server messages

    private func handle(_ msg: ServerMsg) {
        switch msg {
        case .meters(let m):
            meters.inDb = m.inDb; meters.outDb = m.outDb; meters.dsp = m.dsp; meters.xruns = m.xruns
        case .error(let message, let node):
            showBanner(node.map { "\($0): \(message)" } ?? message)
        case .added(let id):
            // The `state` that contains the new pedal arrived just before.
            selectedID = id
        case .param(let node, let param, let value):
            applyRemoteParam(node, param, value)
        case .state(let s):
            if catalog != s.catalog {
                catalog = s.catalog
                catalogByKind = Dictionary(s.catalog.map { ($0.kind, $0) }, uniquingKeysWith: { a, _ in a })
            }
            if presets != s.presets { presets = s.presets }
            if audio != s.audio { audio = s.audio }
            if serverProtocol != s.protocolVersion { serverProtocol = s.protocolVersion }
            if canUndo != s.canUndo { canUndo = s.canUndo }
            if canRedo != s.canRedo { canRedo = s.canRedo }
            if dirty != s.dirty { dirty = s.dirty }
            if !gotState { gotState = true }
            applyGraph(s.graph)
        }
    }

    /// Structure changes only: values are pushed into the existing models when they differ, so a
    /// server echo of what is already on screen invalidates nothing.
    private func applyGraph(_ g: GraphFile) {
        var next: [String: NodeModel] = [:]
        for n in g.node {
            if let m = models[n.id], m.type == n.type {
                // A knob being dragged here keeps its local values: the state that follows the
                // first move of a gesture is older than the finger.
                if m.spec != n, !editedRecently(n.id) { m.spec = n }
                next[n.id] = m
            } else {
                next[n.id] = NodeModel(n)
            }
        }
        if Set(next.keys) != Set(models.keys) || next.contains(where: { models[$0.key] !== $0.value }) { models = next }
        let structure = Chain.derive(g)
        if graph != g { graph = g }
        if chain.nodes.map(\.id) != structure.nodes.map(\.id) || chain.linear != structure.linear { chain = structure }
        if let id = selectedID, !g.node.contains(where: { $0.id == id }) { selectedID = nil }
        if selectedID == nil {
            selectedID = chain.nodes.first { $0.type != "input" && $0.type != "output" }?.id
        }
    }

    /// Another client moved a knob (`param` message): update that node's model only.
    private func applyRemoteParam(_ id: String, _ param: String, _ value: Double) {
        guard let m = models[id], let p = item(kind: m.type)?.params.first(where: { $0.id == param }),
              !editedRecently(id) else { return }
        var spec = m.spec
        if p.tag == .choice {
            guard p.options.indices.contains(Int(value)) else { return }
            spec.params[param] = .string(p.options[Int(value)])
        } else {
            spec.params[param] = .number(value)
        }
        if spec != m.spec { m.spec = spec }
    }

    private func editedRecently(_ id: String) -> Bool {
        guard let t = lastLocalEdit[id] else { return false }
        return Date().timeIntervalSince(t) < 0.5
    }

    func showBanner(_ text: String) {
        banner = text
        bannerTask?.cancel()
        bannerTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(4))
            if !Task.isCancelled { self?.banner = nil }
        }
    }

    // MARK: live edits

    /// Call once when an edit gesture starts, so the whole gesture is one undo step (the core keeps the history).
    func beginEdit() {
        client.send(.editBegin)
    }

    func setNumber(_ id: String, _ p: ParamDesc, _ value: Double) {
        update(id) { $0.params[p.id] = .number(value) }
        queue(.setParam(node: id, param: p.id, value: value), key: id + "." + p.id)
    }

    func setChoice(_ id: String, _ p: ParamDesc, index: Int) {
        guard p.options.indices.contains(index) else { return }
        update(id) { $0.params[p.id] = .string(p.options[index]) }
        queue(.setParam(node: id, param: p.id, value: Double(index)), key: id + "." + p.id)
    }

    func setDefault(_ id: String, _ p: ParamDesc) {
        beginEdit()
        setNumber(id, p, p.def)
    }

    func setBypass(_ id: String, _ bypass: Bool) {
        update(id) { $0.isBypassed = bypass }
        client.send(.setBypass(node: id, bypass: bypass))
    }

    /// Value change: touches only that node's model (no chain derivation, no board redraw).
    private func update(_ id: String, _ change: (inout NodeSpec) -> Void) {
        guard let m = models[id] else { return }
        var spec = m.spec
        change(&spec)
        if spec != m.spec { m.spec = spec }
        lastLocalEdit[id] = Date()
    }

    /// set_param at most every 30 ms per parameter, always ending with the last value.
    private func queue(_ msg: ClientMsg, key: String) {
        pending[key] = msg
        guard !flushScheduled else { return }
        flushScheduled = true
        let wait = max(0, 0.03 - Date().timeIntervalSince(lastFlush))
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            self?.flush()
        }
    }

    private func flush() {
        flushScheduled = false
        lastFlush = Date()
        for m in pending.values { client.send(m) }
        pending.removeAll()
    }

    // MARK: structure and history (the core does the work; this only sends commands)

    func undo() {
        flush()
        client.send(.undo)
    }

    func redo() {
        flush()
        client.send(.redo)
    }

    /// Inserts after the selected pedal (or before the output); the core replies with `added`.
    func addPedal(_ item: CatalogItem) {
        guard chain.linear else { return }
        flush()
        var after: String?
        if let id = selectedID, let n = chain.nodes.first(where: { $0.id == id }), n.type != "output" { after = id }
        client.send(.addNode(kind: item.kind, after: after))
    }

    func removePedal(_ id: String) {
        guard chain.linear, let n = chain.nodes.first(where: { $0.id == id }),
              n.type != "input", n.type != "output" else { return }
        flush()
        client.send(.removeNode(id))
    }

    func movePedal(_ id: String, by delta: Int) {
        let ids = pedalIDs
        guard let i = ids.firstIndex(of: id) else { return }
        moveTo(id, slot: i + delta)
    }

    private var pedalIDs: [String] {
        chain.nodes.filter { $0.type != "input" && $0.type != "output" }.map(\.id)
    }

    /// Moves a pedal to `slot` (0 = first pedal after the input). The chain is reordered on screen
    /// at once; the core's `state` confirms it.
    func moveTo(_ id: String, slot: Int) {
        guard chain.linear else { return }
        var ids = pedalIDs
        guard let i = ids.firstIndex(of: id) else { return }
        let j = min(max(slot, 0), ids.count - 1)
        guard j != i else { return }
        ids.insert(ids.remove(at: i), at: j)
        var nodes = chain.nodes
        let pedals = Dictionary(nodes.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        nodes.replaceSubrange(1..<(nodes.count - 1), with: ids.compactMap { pedals[$0] })
        chain = Chain(nodes: nodes, linear: true)
        selectedID = id
        flush()
        client.send(.moveNode(node: id, to: j))
    }

    /// Every parameter of a node back to its default, as one undo step.
    func resetParams(_ id: String) {
        guard let node = models[id]?.spec, let item = item(for: node) else { return }
        beginEdit()
        for p in item.params {
            switch p.tag {
            case .number:
                setNumber(id, p, p.def)
            case .choice:
                let idx = Int(p.def)
                guard p.options.indices.contains(idx) else { continue }
                setChoice(id, p, index: idx)
            case .file:
                break
            }
        }
    }

    // MARK: presets

    var currentPreset: String { graph.name }

    func loadPreset(_ name: String) {
        client.send(.loadPreset(name))
    }

    func stepPreset(_ delta: Int) {
        guard !presets.isEmpty else { return }
        let i = presets.firstIndex(of: graph.name) ?? (delta > 0 ? -1 : 0)
        let j = (i + delta + presets.count) % presets.count
        loadPreset(presets[j])
    }

    func save() {
        saveAs(graph.name)
    }

    func saveAs(_ rawName: String) {
        let name = rawName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        flush()
        var g = snapshot()
        g.name = name
        client.send(.savePreset(name: name, graph: g))
    }

    /// An empty chain (guitar straight to the amp), sent as a whole graph; the core keeps it as one undo step.
    func newPreset() {
        flush()
        let input = NodeSpec(id: "in", type: "input", pos: [0, 120])
        let output = NodeSpec(id: "out", type: "output", pos: [220, 120])
        client.send(.setGraph(GraphFile(name: "New preset", version: 2, node: [input, output], edge: [EdgeSpec(from: "in", to: "out")])))
        client.send(.hello) // refreshes can_undo / dirty
        selectedID = "out"
    }

    func deletePreset(_ name: String) {
        client.send(.deletePreset(name))
    }
}
