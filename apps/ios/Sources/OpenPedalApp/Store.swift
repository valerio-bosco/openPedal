import Foundation
import Observation
import SwiftUI

/// Input/output levels, kept apart from `Store` so 30 Hz updates only redraw the meters.
@MainActor @Observable
final class MeterModel {
    var inDb: Double = -90
    var outDb: Double = -90
    var dsp: Double = 0
    var xruns: Int = 0
}

/// Latest frequency response from the core (`analyze_pedal`), kept apart so only the designer's graph redraws.
@MainActor @Observable
final class ResponseModel {
    var freq: [Double] = []
    var db: [Double] = []
}

/// One open pedal designer (a full-screen cover): a new pedal merged from board pedals, or an existing one.
struct DesignerSession: Identifiable {
    let id = UUID()
    let model: DesignerModel
    /// Board nodes the pedal replaces on Save (merge); empty when editing a saved pedal.
    let mergeIDs: [String]
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
    /// Custom pedal files (id -> pedal), for the designer.
    private(set) var pedals: [String: PedalFile] = [:]
    var selectedID: String?
    /// Selection mode of the board ("Merge into a pedal"): tapping a pedal marks it instead of selecting it.
    private(set) var selecting = false
    private(set) var selection: Set<String> = []
    var designer: DesignerSession?
    var banner: String?

    let meters = MeterModel()
    let analysis = ResponseModel()

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
    private var pendingSave: (id: String, done: (Bool) -> Void)?
    private var pendingSelect: String?

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
            pendingSelect = nil
            if let p = pendingSave { pendingSave = nil; p.done(false) }
        case .added(let id):
            // The `state` that contains the new pedal arrived just before.
            selectedID = id
        case .param(let node, let param, let value):
            applyRemoteParam(node, param, value)
        case .response(let freq, let db):
            analysis.freq = freq
            analysis.db = db
        case .state(let s):
            if catalog != s.catalog {
                catalog = s.catalog
                catalogByKind = Dictionary(s.catalog.map { ($0.kind, $0) }, uniquingKeysWith: { a, _ in a })
            }
            if presets != s.presets { presets = s.presets }
            if pedals != s.pedals { pedals = s.pedals }
            if audio != s.audio { audio = s.audio }
            if serverProtocol != s.protocolVersion { serverProtocol = s.protocolVersion }
            if canUndo != s.canUndo { canUndo = s.canUndo }
            if canRedo != s.canRedo { canRedo = s.canRedo }
            if dirty != s.dirty { dirty = s.dirty }
            if !gotState { gotState = true }
            applyGraph(s.graph)
            // The reply to `save_pedal` (the file is in `pedals` now).
            if let p = pendingSave, s.pedals[p.id] != nil { pendingSave = nil; p.done(true) }
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
        if let id = pendingSelect, g.node.contains(where: { $0.id == id }) { selectedID = id; pendingSelect = nil }
        if !selection.isEmpty { selection = selection.filter { id in g.node.contains { $0.id == id } } }
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

    // MARK: custom pedals

    /// Frequency response of a draft pedal; the core answers with `response` (or `error`).
    func analyze(_ pedal: PedalFile, values: [String: Double]) {
        client.send(.analyzePedal(pedal: pedal, values: values))
    }

    /// Writes `pedals/<id>.toml`. `done(true)` runs when the core confirms with its new `state`, `done(false)` on `error`.
    func savePedal(id: String, pedal: PedalFile, done: @escaping (Bool) -> Void) {
        flush()
        pendingSave = (id, done)
        client.send(.savePedal(id: id, pedal: pedal))
    }

    /// Opens the designer on the selected board pedals, in chain order (they must be neighbours).
    func openMergeDesigner() {
        guard canMergeSelection else { return }
        let ids = selectedPedalIDs
        let specs = currentOrder().filter { ids.contains($0.id) }
        designer = DesignerSession(model: DesignerModel(store: self, merging: specs), mergeIDs: ids)
    }

    /// Opens the designer on a saved custom pedal used by a board node.
    func openEditDesigner(node id: String) {
        guard let type = models[id]?.type, type.hasPrefix(OpenPedalProtocol.pedalPrefix) else { return }
        let pedalID = String(type.dropFirst(OpenPedalProtocol.pedalPrefix.count))
        guard let file = pedals[pedalID] else { showBanner("Pedal file \"\(pedalID)\" not found"); return }
        designer = DesignerSession(model: DesignerModel(store: self, editing: pedalID, file: file), mergeIDs: [])
    }

    /// Replaces the neighbouring pedals `ids` by one `pedal:<pedalID>` node. One `set_graph` of the
    /// rewritten chain, so the core records a single undo step; `hello` brings the new state back.
    func mergeIntoPedal(ids: [String], pedalID: String) {
        guard chain.linear else { return }
        flush()
        let order = currentOrder()
        let idx = order.indices.filter { ids.contains(order[$0].id) }
        guard let first = idx.first, let last = idx.last, idx.count == ids.count, last - first + 1 == idx.count else { return }
        let taken = Set(order.map(\.id))
        var newID = pedalID, n = 2
        while taken.contains(newID) { newID = "\(pedalID)\(n)"; n += 1 }
        var next = order
        next.replaceSubrange(first...last, with: [NodeSpec(id: newID, type: OpenPedalProtocol.pedalPrefix + pedalID)])
        client.send(.setGraph(Chain.rewrite(snapshot(), order: next)))
        client.send(.hello)
        pendingSelect = newID
        endSelecting()
    }

    // MARK: selection mode

    func beginSelecting(_ first: String? = nil) {
        guard chain.linear else { return }
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            selecting = true
            selection = first.map { [$0] } ?? []
        }
    }

    func endSelecting() {
        withAnimation(.spring(response: 0.4, dampingFraction: 0.85)) {
            selecting = false
            selection = []
        }
    }

    func toggleSelected(_ id: String) {
        if selection.contains(id) { selection.remove(id) } else { selection.insert(id) }
    }

    /// Selected pedals in chain order.
    var selectedPedalIDs: [String] { pedalIDs.filter { selection.contains($0) } }

    /// At least one pedal, and no unselected pedal between the selected ones.
    var canMergeSelection: Bool {
        let ids = pedalIDs
        let marks = ids.indices.filter { selection.contains(ids[$0]) }
        guard let first = marks.first, let last = marks.last else { return false }
        return last - first + 1 == marks.count
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
