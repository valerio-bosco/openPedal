import Foundation

/// WebSocket client for the in-process server (ws://127.0.0.1:<port>/ws). Reconnects forever.
@MainActor
final class WSClient {
    var onMessage: ((ServerMsg) -> Void)?
    var onStatus: ((Bool) -> Void)?

    private let url: URL
    private var task: URLSessionWebSocketTask?
    private var loop: Task<Void, Never>?

    init(port: UInt16) {
        url = URL(string: "ws://127.0.0.1:\(port)/ws")!
    }

    func start() {
        loop?.cancel()
        loop = Task { [weak self] in
            while !Task.isCancelled {
                await self?.session()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    func stop() {
        loop?.cancel()
        task?.cancel(with: .goingAway, reason: nil)
    }

    func send(_ msg: ClientMsg) {
        guard let text = msg.json() else { return }
        task?.send(.string(text)) { _ in }
    }

    private func session() async {
        let t = URLSession.shared.webSocketTask(with: url)
        t.maximumMessageSize = 16 * 1024 * 1024
        task = t
        t.resume()
        send(.hello)
        var opened = false
        while !Task.isCancelled {
            do {
                let raw = try await t.receive()
                if !opened { opened = true; onStatus?(true) }
                let data: Data
                switch raw {
                case .string(let s): data = Data(s.utf8)
                case .data(let d): data = d
                @unknown default: continue
                }
                if let decoded = try? ServerMsg.decode(data) { onMessage?(decoded) }
            } catch {
                break
            }
        }
        onStatus?(false)
        t.cancel(with: .goingAway, reason: nil)
    }
}
