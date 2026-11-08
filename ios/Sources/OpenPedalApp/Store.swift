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

@MainActor @Observable
final class Store {
    private(set) var connected = false
    private(set) var catalog: [CatalogItem] = []
    private(set) var graph = GraphFile(name: "", version: 2, node: [], edge: [])
    private(set) var presets: [String] = []
    private(set) var audio: AudioInfo?
    private(set) var serverProtocol: UInt32?
    private(set) var gotState = false
    private(set) var dirty = false
    private(set) var undoStack: [GraphFile] = []
    private(set) var redoStack: [GraphFile] = []
    private(set) var chain = Chain(nodes: [], linear: true)
    var selectedID: String?
    var banner: String?

    let meters = MeterModel()

    /// The server speaks a newer contract than this app knows.
    var protocolTooNew: Bool { (serverProtocol ?? 0) > supportedProtocolVersion }
    var canEditChain: Bool { chain.linear }
    /// At least one node besides the input and output jacks.
    var hasPedals: Bool { chain.nodes.contains { $0.type != "input" && $0.type != "output" } }
    var canUndo: Bool { !undoStack.isEmpty }
    var canRedo: Bool { !redoStack.isEmpty }

    private let client: WSClient
    private var expectLoad = false
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

    func item(for node: NodeSpec) -> CatalogItem? { catalog.first { $0.kind == node.type } }
    var selected: NodeSpec? { graph.node.first { $0.id == selectedID } }

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
        case .error(let e):
            showBanner(e.node.map { "\($0): \(e.message)" } ?? e.message)
        case .state(let s):
            catalog = s.catalog
            presets = s.presets
            audio = s.audio
            serverProtocol = s.protocolVersion
            if !gotState || expectLoad || s.graph.name != graph.name {
                dirty = false
                undoStack = []; redoStack = []
            }
            gotState = true
            expectLoad = false
            applyGraph(s.graph)
        }
    }

    private func applyGraph(_ g: GraphFile) {
        graph = g
        chain = Chain.derive(g)
        if let id = selectedID, !g.node.contains(where: { $0.id == id }) { selectedID = nil }
        if selectedID == nil {
            selectedID = chain.nodes.first { $0.type != "input" && $0.type != "output" }?.id
        }
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

    /// Call once when an edit gesture starts, so the whole gesture is one undo step.
    func beginEdit() {
        pushUndo()
    }

    func setNumber(_ node: NodeSpec, _ p: ParamDesc, _ value: Double) {
        update(node.id) { $0.params[p.id] = .number(value) }
        queue(.setParam(node: node.id, param: p.id, value: value), key: node.id + "." + p.id)
    }

    func setChoice(_ node: NodeSpec, _ p: ParamDesc, index: Int) {
        guard p.options.indices.contains(index) else { return }
        pushUndo()
        update(node.id) { $0.params[p.id] = .string(p.options[index]) }
        queue(.setParam(node: node.id, param: p.id, value: Double(index)), key: node.id + "." + p.id)
    }

    func setDefault(_ node: NodeSpec, _ p: ParamDesc) {
        pushUndo()
        setNumber(node, p, p.def)
    }

    func setBypass(_ id: String, _ bypass: Bool) {
        pushUndo()
        update(id) { $0.bypass = bypass }
        client.send(.setBypass(node: id, bypass: bypass))
    }

    private func update(_ id: String, _ change: (inout NodeSpec) -> Void) {
        guard let i = graph.node.firstIndex(where: { $0.id == id }) else { return }
        change(&graph.node[i])
        chain = Chain.derive(graph)
        dirty = true
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

    // MARK: structure (chain only)

    private func pushUndo() {
        undoStack.append(graph)
        if undoStack.count > 100 { undoStack.removeFirst() }
        redoStack = []
        dirty = true
    }

    private func commit(_ g: GraphFile) {
        flush()
        applyGraph(g)
        dirty = true
        client.send(.setGraph(g))
    }

    func undo() {
        guard let prev = undoStack.popLast() else { return }
        redoStack.append(graph)
        commit(prev)
    }

    func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(graph)
        commit(next)
    }

    func addPedal(_ item: CatalogItem) {
        guard chain.linear else { return }
        var order = chain.nodes
        let at: Int
        if let id = selectedID, let i = order.firstIndex(where: { $0.id == id }), order[i].type != "output" {
            at = i + 1
        } else {
            at = max(order.count - 1, 0)
        }
        var id = item.kind.replacingOccurrences(of: "pedal:", with: "")
        var n = 2
        let base = id
        while graph.node.contains(where: { $0.id == id }) { id = "\(base)\(n)"; n += 1 }
        order.insert(NodeSpec(id: id, type: item.kind), at: min(at, max(order.count - 1, 0)))
        pushUndo()
        selectedID = id
        commit(Chain.rewrite(graph, order: order))
    }

    func removePedal(_ id: String) {
        guard chain.linear, let n = graph.node.first(where: { $0.id == id }),
              n.type != "input", n.type != "output" else { return }
        pushUndo()
        commit(Chain.rewrite(graph, order: chain.nodes.filter { $0.id != id }))
    }

    func movePedal(_ id: String, by delta: Int) {
        guard let i = chain.nodes.firstIndex(where: { $0.id == id }) else { return }
        moveTo(id, slot: (i - 1) + delta)
    }

    /// Moves a pedal to `slot` (0 = first pedal after the input). Input stays first, output last.
    func moveTo(_ id: String, slot: Int) {
        guard chain.linear else { return }
        var order = chain.nodes
        guard let i = order.firstIndex(where: { $0.id == id }), i >= 1, i <= order.count - 2 else { return }
        let j = min(max(slot, 0), order.count - 3) + 1
        guard j != i else { return }
        pushUndo()
        let node = order.remove(at: i)
        order.insert(node, at: j)
        selectedID = id
        commit(Chain.rewrite(graph, order: order))
    }

    /// Every parameter of a node back to its default, as one undo step.
    func resetParams(_ node: NodeSpec) {
        guard let item = item(for: node) else { return }
        pushUndo()
        for p in item.params {
            switch p.kind {
            case .number:
                setNumber(node, p, p.def)
            case .choice:
                let idx = Int(p.def)
                guard p.options.indices.contains(idx) else { continue }
                update(node.id) { $0.params[p.id] = .string(p.options[idx]) }
                queue(.setParam(node: node.id, param: p.id, value: Double(idx)), key: node.id + "." + p.id)
            case .file:
                break
            }
        }
    }

    // MARK: presets

    var currentPreset: String { graph.name }

    func loadPreset(_ name: String) {
        expectLoad = true
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
        graph.name = name
        client.send(.savePreset(name: name, graph: graph))
        if !presets.contains(name) { presets.append(name); presets.sort() }
        dirty = false
    }

    func newPreset() {
        let input = NodeSpec(id: "in", type: "input")
        let output = NodeSpec(id: "out", type: "output")
        let g = Chain.rewrite(GraphFile(name: "New preset", version: 2, node: [], edge: []), order: [input, output])
        pushUndo()
        commit(g)
        selectedID = "out"
    }

    func deletePreset(_ name: String) {
        client.send(.deletePreset(name))
        presets.removeAll { $0 == name }
    }
}
