import Foundation

/// Signal-ordered view of a graph (docs/PLAN.md §10.1).
struct Chain {
    var nodes: [NodeSpec]
    /// true: a simple line input -> ... -> output covering every node (adding/reordering allowed).
    var linear: Bool

    static func derive(_ g: GraphFile) -> Chain {
        let byID = Dictionary(g.node.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var next: [String: [String]] = [:]
        var indeg: [String: Int] = [:]
        for e in g.edge where byID[e.from] != nil && byID[e.to] != nil {
            next[e.from, default: []].append(e.to)
            indeg[e.to, default: 0] += 1
        }
        // Linear: every node has at most one in and one out edge, and the walk from the input
        // reaches every node and ends at the output.
        let inputs = g.node.filter { $0.type == "input" }
        if inputs.count == 1,
           next.values.allSatisfy({ $0.count <= 1 }), indeg.values.allSatisfy({ $0 <= 1 }) {
            var order: [NodeSpec] = []
            var seen = Set<String>()
            var cur: String? = inputs[0].id
            while let id = cur, !seen.contains(id), let n = byID[id] {
                seen.insert(id); order.append(n); cur = next[id]?.first
            }
            if order.count == g.node.count, order.last?.type == "output" {
                return Chain(nodes: order, linear: true)
            }
        }
        // Otherwise: topological order (Kahn), ties by x position, cycles appended at the end.
        var deg = indeg
        var ready = g.node.filter { (deg[$0.id] ?? 0) == 0 }
        var out: [NodeSpec] = []
        var done = Set<String>()
        while !ready.isEmpty {
            ready.sort { ($0.pos.first ?? 0) < ($1.pos.first ?? 0) }
            let n = ready.removeFirst()
            out.append(n); done.insert(n.id)
            for to in next[n.id] ?? [] {
                deg[to, default: 0] -= 1
                if deg[to] == 0, let m = byID[to] { ready.append(m) }
            }
        }
        out += g.node.filter { !done.contains($0.id) }
        return Chain(nodes: out, linear: false)
    }

    /// Rebuilds node positions and edges for a linear order.
    static func rewrite(_ g: GraphFile, order: [NodeSpec]) -> GraphFile {
        var g = g
        var nodes = order
        for i in nodes.indices { nodes[i].pos = [Double(i) * 220, 120] }
        g.node = nodes
        g.edge = zip(nodes, nodes.dropFirst()).map { EdgeSpec(from: $0.id, to: $1.id) }
        return g
    }
}
