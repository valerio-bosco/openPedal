import AVFoundation
import SwiftUI

/// Recording of the dry guitar input, implemented by `AudioHost` (another branch).
/// Writes `url` (WAV, guitar channel on the left) after `countIn` seconds and lasts `seconds`.
@MainActor
protocol AudioHostRecording: AnyObject {
    func recordDI(to url: URL, seconds: Double, countIn: Double) async throws
}

struct RecordingUnavailable: LocalizedError {
    var errorDescription: String? { "Recording is not available in this build." }
}

private struct AudioHostRecordingKey: EnvironmentKey {
    static let defaultValue: (any AudioHostRecording)? = nil
}

extension EnvironmentValues {
    /// The recorder Tone Lab uses for "Record my take". Without one, recording throws "not available".
    var audioHostRecording: (any AudioHostRecording)? {
        get { self[AudioHostRecordingKey.self] }
        set { self[AudioHostRecordingKey.self] = newValue }
    }
}

/// Where Tone Lab keeps its files (same data dir as AudioHost: Application Support/OpenPedal).
enum ToneLabPaths {
    static var dataDir: URL {
        let base = (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("OpenPedal", isDirectory: true)
    }
    static var jobsDir: URL { dataDir.appendingPathComponent("tonelab/jobs", isDirectory: true) }
    static func jobDir(_ id: String) -> URL { jobsDir.appendingPathComponent(id, isDirectory: true) }
    static func mix(_ id: String) -> URL { jobDir(id).appendingPathComponent("mix.wav") }
    static func partDir(_ job: String, _ part: String) -> URL { jobDir(job).appendingPathComponent("parts/\(part)", isDirectory: true) }
    static func target(_ job: String, _ part: String) -> URL { partDir(job, part).appendingPathComponent("target.wav") }
    static func render(_ job: String, _ part: String) -> URL { partDir(job, part).appendingPathComponent("fit/render.wav") }
    static func userDI(_ job: String, _ part: String) -> URL { partDir(job, part).appendingPathComponent("di-user.wav") }

    static func slug(_ s: String) -> String {
        let folded = s.folding(options: [.diacriticInsensitive, .caseInsensitive], locale: nil)
        var out = ""
        var dash = false
        for ch in folded.unicodeScalars {
            if ch.isASCII, CharacterSet.alphanumerics.contains(ch) { out.unicodeScalars.append(ch); dash = false }
            else if !dash, !out.isEmpty { out.append("-"); dash = true }
        }
        out = out.trimmingCharacters(in: CharacterSet(charactersIn: "-"))
        return out.isEmpty ? "song" : String(out.prefix(40))
    }

    /// `yyyy-MM-dd-<slug>`, with a numeric suffix if the folder already exists.
    static func newJobID(title: String) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        let base = "\(f.string(from: Date()))-\(slug(title))"
        var id = base, n = 2
        while FileManager.default.fileExists(atPath: jobDir(id).path) { id = "\(base)-\(n)"; n += 1 }
        return id
    }
}

struct ToneLabImported: Sendable {
    let id: String
    let title: String
    let mix: URL
}

enum ToneLabImportError: LocalizedError {
    case empty, noAccess
    var errorDescription: String? {
        switch self {
        case .empty: "The file has no audio."
        case .noAccess: "The app cannot read this file."
        }
    }
}

enum ToneLabImporter {
    /// Decodes `source` and writes `<jobs>/<id>/mix.wav` as float32 stereo at the file's own rate.
    /// Runs off the main actor; `progress` gets 0...1.
    static func importAudio(from source: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> ToneLabImported {
        try await Task.detached(priority: .userInitiated) {
            let scoped = source.startAccessingSecurityScopedResource()
            defer { if scoped { source.stopAccessingSecurityScopedResource() } }
            let title = source.deletingPathExtension().lastPathComponent
            let input: AVAudioFile
            do { input = try AVAudioFile(forReading: source) } catch { throw ToneLabImportError.noAccess }
            let total = input.length
            guard total > 0 else { throw ToneLabImportError.empty }
            let id = ToneLabPaths.newJobID(title: title)
            let dir = ToneLabPaths.jobDir(id)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let mix = dir.appendingPathComponent("mix.wav")
            let rate = input.processingFormat.sampleRate
            guard let outFmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 2, interleaved: false) else { throw ToneLabImportError.empty }
            do {
                let out = try AVAudioFile(forWriting: mix, settings: outFmt.settings, commonFormat: .pcmFormatFloat32, interleaved: false)
                let chunk: AVAudioFrameCount = 65536
                guard let inBuf = AVAudioPCMBuffer(pcmFormat: input.processingFormat, frameCapacity: chunk),
                      let outBuf = AVAudioPCMBuffer(pcmFormat: outFmt, frameCapacity: chunk) else { throw ToneLabImportError.empty }
                let inCh = Int(input.processingFormat.channelCount)
                while input.framePosition < total {
                    try Task.checkCancellation()
                    try input.read(into: inBuf, frameCount: chunk)
                    let n = Int(inBuf.frameLength)
                    if n == 0 { break }
                    guard let src = inBuf.floatChannelData, let dst = outBuf.floatChannelData else { break }
                    // Mono is copied to both sides; more than two channels keep the first two.
                    memcpy(dst[0], src[0], n * MemoryLayout<Float>.size)
                    memcpy(dst[1], src[inCh > 1 ? 1 : 0], n * MemoryLayout<Float>.size)
                    outBuf.frameLength = AVAudioFrameCount(n)
                    try out.write(from: outBuf)
                    progress(min(1, Double(input.framePosition) / Double(total)))
                }
            } catch {
                try? FileManager.default.removeItem(at: dir)
                throw error
            }
            progress(1)
            return ToneLabImported(id: id, title: title, mix: mix)
        }.value
    }

    /// Max-abs peaks of a WAV, `buckets` of them, and its duration in seconds.
    static func peaks(of url: URL, buckets: Int = 1600) async -> (peaks: [Float], duration: Double)? {
        await Task.detached(priority: .utility) { () -> (peaks: [Float], duration: Double)? in
            guard let file = try? AVAudioFile(forReading: url), file.length > 0 else { return nil }
            let total = Int(file.length)
            let per = max(1, total / buckets)
            var out = [Float](repeating: 0, count: (total + per - 1) / per)
            let chunk: AVAudioFrameCount = 32768
            guard let buf = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk) else { return nil }
            var pos = 0
            let ch = Int(file.processingFormat.channelCount)
            while pos < total {
                if Task.isCancelled { return nil }
                guard (try? file.read(into: buf, frameCount: chunk)) != nil, buf.frameLength > 0, let data = buf.floatChannelData else { break }
                for i in 0..<Int(buf.frameLength) {
                    var m: Float = 0
                    for c in 0..<ch { m = max(m, abs(data[c][i])) }
                    let b = min(out.count - 1, (pos + i) / per)
                    if m > out[b] { out[b] = m }
                }
                pos += Int(buf.frameLength)
            }
            let top = max(out.max() ?? 1, 0.0001)
            return (out.map { $0 / top }, Double(total) / file.processingFormat.sampleRate)
        }.value
    }
}

func tlClock(_ s: Double) -> String {
    let t = max(0, s)
    return "\(Int(t) / 60):" + String(format: "%02d", Int(t) % 60)
}

func tlDb(_ x: Double) -> String { String(format: "%.1f dB", x) }

func tlTickStep(_ duration: Double) -> Double {
    for s in [1.0, 2, 5, 10, 15, 30, 60, 120, 300] where duration / s <= 14 { return s }
    return 600
}

let tlKindColor: [String: Color] = [
    "centre": Color(hex: 0x4cc38a), "double": Color(hex: 0xffb23f), "wide": Color(hex: 0x5aa9ff), "none": Color(hex: 0x3a3e4a),
]
let tlKindText: [String: String] = [
    "centre": "Centre: one guitar in the middle (easiest to copy)",
    "double": "Double: the same part on both sides",
    "wide": "Wide: spread across the stereo field",
    "none": "No guitar",
]
