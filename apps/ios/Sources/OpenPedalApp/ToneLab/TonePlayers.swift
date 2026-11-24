import AVFoundation
import Observation

/// Plays a stretch of one WAV with AVAudioPlayer and stops at its end.
@MainActor @Observable
final class RegionPlayer {
    private(set) var playing = false
    private(set) var position: Double = 0
    private var player: AVAudioPlayer?
    private var url: URL?
    private var end: Double = 0
    private var ticker: Task<Void, Never>?

    func play(_ url: URL, from start: Double, to end: Double) {
        stop()
        guard let p = try? AVAudioPlayer(contentsOf: url) else { return }
        player = p; self.url = url; self.end = end
        p.currentTime = start
        p.prepareToPlay()
        p.play()
        playing = true
        position = start
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(33))
                guard let self, let p = self.player else { return }
                self.position = p.currentTime
                if p.currentTime >= self.end || !p.isPlaying { self.stop(); return }
            }
        }
    }

    func stop() {
        ticker?.cancel(); ticker = nil
        player?.stop(); player = nil
        playing = false
    }
}

/// A/B player: two files of the same length, switching keeps the position.
@MainActor @Observable
final class ABPlayerModel {
    enum Side { case target, result }
    private(set) var side: Side = .result
    private(set) var playing = false
    private(set) var position: Double = 0
    private(set) var duration: Double = 0
    private(set) var ready = false
    private var target: AVAudioPlayer?
    private var result: AVAudioPlayer?
    private var ticker: Task<Void, Never>?

    func load(target t: URL, result r: URL) {
        stop()
        target = try? AVAudioPlayer(contentsOf: t)
        result = try? AVAudioPlayer(contentsOf: r)
        target?.prepareToPlay(); result?.prepareToPlay()
        duration = max(target?.duration ?? 0, result?.duration ?? 0)
        ready = target != nil && result != nil
        position = 0
    }

    private var current: AVAudioPlayer? { side == .target ? target : result }
    private var other: AVAudioPlayer? { side == .target ? result : target }

    func toggle() {
        if playing { pause() } else { play() }
    }

    func play() {
        guard let c = current else { return }
        c.currentTime = position
        c.play()
        playing = true
        ticker?.cancel()
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(33))
                guard let self, let c = self.current else { return }
                if c.isPlaying { self.position = c.currentTime }
                else { self.playing = false; if c.currentTime >= self.duration - 0.05 { self.position = 0 }; return }
            }
        }
    }

    func pause() {
        ticker?.cancel(); ticker = nil
        current?.pause(); other?.pause()
        playing = false
    }

    func stop() { pause(); position = 0; target = nil; result = nil; ready = false }

    func seek(_ t: Double) {
        position = t
        current?.currentTime = t
    }

    func choose(_ s: Side) {
        guard s != side else { return }
        let was = playing
        position = current?.currentTime ?? position
        current?.pause()
        side = s
        if was { play() }
    }
}
