import AVFoundation
import Foundation

/// Records the dry guitar (the channel AudioHost feeds to the core) to a WAV file.
protocol AudioHostRecording {
    /// Waits `countIn` seconds, then records `seconds` of the dry guitar into `url`
    /// (float32 mono WAV, 48 kHz). The guitar keeps playing through the current preset.
    func recordDI(to url: URL, seconds: Double, countIn: Double) async throws
}

enum RecorderError: LocalizedError {
    case notRunning, busy, invalidDuration, writeFailed(String)
    var errorDescription: String? {
        switch self {
        case .notRunning: "The audio engine is not running."
        case .busy: "A recording is already in progress."
        case .invalidDuration: "Invalid recording length."
        case .writeFailed(let m): "Could not write the recording: \(m)"
        }
    }
}

/// Preallocated capture buffer shared with the audio thread. The audio thread only calls `push`:
/// plain stores into memory allocated up front plus one atomic-style index publish (no allocation,
/// no locks, no file IO). Everything else runs on the main/background side.
final class DIRecorder: @unchecked Sendable {
    let capacity: Int
    let sampleRate: Double
    let samples: UnsafeMutablePointer<Float>
    /// 0 = idle, 1 = armed (capturing). Written by the control side, read by the audio thread.
    private let flag = UnsafeMutablePointer<Int32>.allocate(capacity: 1)
    /// Frames written so far; only the audio thread writes it, the control side polls it.
    private let written = UnsafeMutablePointer<Int>.allocate(capacity: 1)

    init(seconds: Double, sampleRate: Double) {
        self.sampleRate = sampleRate
        capacity = max(1, Int((seconds * sampleRate).rounded(.up)))
        samples = .allocate(capacity: capacity)
        samples.initialize(repeating: 0, count: capacity)
        flag.initialize(to: 0)
        written.initialize(to: 0)
    }

    deinit {
        samples.deallocate()
        flag.deallocate()
        written.deallocate()
    }

    func arm() { OSMemoryBarrier(); flag.pointee = 1; OSMemoryBarrier() }
    func disarm() { flag.pointee = 0; OSMemoryBarrier() }
    var count: Int { OSMemoryBarrier(); return written.pointee }
    var isFull: Bool { count >= capacity }

    /// Audio thread. Copies up to the remaining capacity; ignores the rest.
    @inline(__always)
    func push(_ input: UnsafePointer<Float>, frames: Int) {
        guard flag.pointee == 1 else { return }
        let n = written.pointee
        let take = min(frames, capacity - n)
        guard take > 0 else { return }
        (samples + n).update(from: input, count: take)
        OSMemoryBarrier() // data before index
        written.pointee = n + take
    }
}

extension AudioHost: AudioHostRecording {
    func recordDI(to url: URL, seconds: Double, countIn: Double) async throws {
        guard seconds > 0, seconds <= 600, countIn >= 0 else { throw RecorderError.invalidDuration }
        guard isRunning else { throw RecorderError.notRunning }
        guard activeRecorder == nil else { throw RecorderError.busy }
        let rate = currentSampleRate
        let rec = DIRecorder(seconds: seconds, sampleRate: rate)
        try await Task.sleep(for: .seconds(countIn))
        activeRecorder = rec          // the audio thread picks it up on its next buffer
        rec.arm()
        defer { rec.disarm(); activeRecorder = nil }
        let deadline = Date().addingTimeInterval(seconds + 2)
        while !rec.isFull {
            try await Task.sleep(for: .milliseconds(20))
            if Date() > deadline { break }   // engine stalled or rebuilt: keep what we have
        }
        rec.disarm()
        activeRecorder = nil          // unpublish, then give an in-flight callback time to finish
        try await Task.sleep(for: .milliseconds(50))
        let frames = rec.count
        guard frames > 0 else { throw RecorderError.writeFailed("no audio captured") }
        let data = UnsafeBufferPointer(start: rec.samples, count: frames)
        try await Task.detached(priority: .userInitiated) {
            try Self.writeWAV(Array(data), sourceRate: rate, to: url)
        }.value
    }

    nonisolated static func writeWAV(_ mono: [Float], sourceRate: Double, to url: URL) throws {
        let target = 48000.0
        var out = mono
        if abs(sourceRate - target) > 0.5 {
            // Linear interpolation: DI material, only used for matching/analysis.
            let ratio = sourceRate / target
            let n = Int(Double(mono.count) / ratio)
            out = (0..<n).map { i in
                let pos = Double(i) * ratio
                let i0 = min(Int(pos), mono.count - 1), i1 = min(i0 + 1, mono.count - 1)
                let f = Float(pos - Double(i0))
                return mono[i0] * (1 - f) + mono[i1] * f
            }
        }
        do {
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let settings: [String: Any] = [
                AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: target, AVNumberOfChannelsKey: 1,
                AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true,
                AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false,
            ]
            let file = try AVAudioFile(forWriting: url, settings: settings,
                                       commonFormat: .pcmFormatFloat32, interleaved: false)
            let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: target, channels: 1, interleaved: false)!
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(out.count))!
            buffer.frameLength = AVAudioFrameCount(out.count)
            out.withUnsafeBufferPointer { buffer.floatChannelData![0].update(from: $0.baseAddress!, count: out.count) }
            try file.write(from: buffer)
        } catch {
            throw RecorderError.writeFailed(error.localizedDescription)
        }
    }
}
