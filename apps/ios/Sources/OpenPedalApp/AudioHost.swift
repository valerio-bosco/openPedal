import AVFoundation
import Foundation
import OpenPedalCore

/// Owns the audio session and the audio graph; the Rust core does all the DSP.
///
/// Audio path (all inside one AVAudioEngine, so input and output share the hardware cycle):
///   inputNode -> AVAudioSinkNode  (copies the guitar channel into `inputBuffer`)
///   AVAudioSourceNode (calls openpedal_process, writes stereo) -> mainMixer -> output
@MainActor
final class AudioHost: ObservableObject {
    enum State {
        case starting
        case running(port: UInt16)
        case failed(String)
    }

    @Published private(set) var state: State = .starting

    /// Largest buffer we expect from iOS (it is normally 64-512 frames).
    private static let maxFrames: UInt32 = 4096
    /// Which input channel carries the guitar on a multi-channel interface.
    /// Scarlett Solo: channel 2 (the Inst jack) = index 1. Mono devices use index 0.
    private static let guitarChannel = 1

    private var core: OpaquePointer?
    private let engine = AVAudioEngine()
    private var inputBuffer: UnsafeMutablePointer<Float>?
    private var started = false

    func start() {
        guard !started else { return }
        started = true
        state = .starting
        AVAudioApplication.requestRecordPermission { granted in
            Task { @MainActor in
                if granted {
                    self.bringUp()
                } else {
                    self.fail("Microphone access is needed to read the audio interface. Enable it in Settings > openPedal.")
                }
            }
        }
    }

    private func fail(_ message: String) {
        started = false
        state = .failed(message)
    }

    private func bringUp() {
        do {
            let session = AVAudioSession.sharedInstance()
            // .measurement turns off the system's input processing (AGC, EQ, noise reduction).
            try session.setCategory(.playAndRecord, mode: .measurement, options: [.allowBluetoothA2DP])
            try session.setPreferredSampleRate(48000)
            try session.setPreferredIOBufferDuration(0.0027) // ~128 frames at 48 kHz
            try session.setActive(true)
            // Prefer the USB interface for input when it is connected.
            if let usb = session.availableInputs?.first(where: { $0.portType == .usbAudio }) {
                try session.setPreferredInput(usb)
            }

            let sampleRate = session.sampleRate
            let dataDir = try Self.prepareDataDir()
            let factory = Self.factoryDir()
            let started: OpaquePointer? = dataDir.withCString { data in
                if let factory { return factory.withCString { openpedal_start2(Float(sampleRate), Self.maxFrames, data, $0, 0) } }
                return openpedal_start2(Float(sampleRate), Self.maxFrames, data, nil, 0)
            }
            guard let handle = started else {
                let reason = openpedal_last_error().map { String(cString: $0) } ?? "unknown error"
                fail("Core failed to start: \(reason)")
                return
            }
            core = handle
            try buildGraph(handle: handle, sampleRate: sampleRate)
            try engine.start()
            state = .running(port: openpedal_port(handle))
        } catch {
            fail("\(error.localizedDescription)")
        }
    }

    private func buildGraph(handle: OpaquePointer, sampleRate: Double) throws {
        // Preallocated here, used from the audio thread: no allocation in the callbacks.
        let capacity = Int(Self.maxFrames)
        let input = UnsafeMutablePointer<Float>.allocate(capacity: capacity)
        input.initialize(repeating: 0, count: capacity)
        inputBuffer = input
        let channel = Self.guitarChannel
        let maxFrames = Self.maxFrames

        let inputFormat = engine.inputNode.outputFormat(forBus: 0)
        let channels = Int(inputFormat.channelCount)
        let pickChannel = min(channel, max(channels - 1, 0))

        // Input: copy the guitar channel. Captures only values and raw pointers (no `self`).
        let sink = AVAudioSinkNode { _, frameCount, bufferList in
            let frames = min(frameCount, maxFrames)
            let abl = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: bufferList))
            if abl.count > 1 {
                // Non-interleaved: one buffer per channel.
                let b = abl[min(pickChannel, abl.count - 1)]
                if let data = b.mData?.assumingMemoryBound(to: Float.self) {
                    input.update(from: data, count: Int(frames))
                }
            } else if let b = abl.first, let data = b.mData?.assumingMemoryBound(to: Float.self) {
                // Interleaved: stride by the number of channels in the buffer.
                let stride = max(Int(b.mNumberChannels), 1)
                let c = min(pickChannel, stride - 1)
                for i in 0..<Int(frames) { input[i] = data[i * stride + c] }
            }
            return noErr
        }

        // Output: stereo, non-interleaved float.
        let outFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 2, interleaved: false)!
        let source = AVAudioSourceNode(format: outFormat) { isSilence, _, frameCount, bufferList in
            let abl = UnsafeMutableAudioBufferListPointer(bufferList)
            guard abl.count >= 2,
                  let left = abl[0].mData?.assumingMemoryBound(to: Float.self),
                  let right = abl[1].mData?.assumingMemoryBound(to: Float.self),
                  frameCount <= maxFrames else {
                isSilence.pointee = true
                return noErr
            }
            openpedal_process(handle, input, left, right, frameCount)
            return noErr
        }

        engine.attach(sink)
        engine.attach(source)
        engine.connect(engine.inputNode, to: sink, format: inputFormat)
        engine.connect(source, to: engine.mainMixerNode, format: outFormat)
    }

    /// `Application Support/OpenPedal`: the core's data dir (`presets/`, `pedals/`). The core
    /// seeds it from the bundled factory content; see `openpedal_start2`.
    private static func prepareDataDir() throws -> String {
        let fm = FileManager.default
        let base = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("OpenPedal", isDirectory: true)
        try fm.createDirectory(at: base, withIntermediateDirectories: true)
        migrateSeededRecord(into: base)
        return base.path
    }

    /// The bundle's resource folder, which holds the factory `presets/` and `pedals/`.
    private static func factoryDir() -> String? {
        Bundle.module.url(forResource: "presets", withExtension: nil)?.deletingLastPathComponent().path
    }

    /// Older versions seeded in Swift and remembered the copied files in UserDefaults. Hand
    /// that record to the core's `.seeded.toml` once (so factory presets the user deleted stay
    /// deleted), then stop using the keys.
    private static func migrateSeededRecord(into base: URL) {
        let defaults = UserDefaults.standard
        let kinds = ["presets", "pedals"]
        let state = base.appendingPathComponent(".seeded.toml")
        guard !FileManager.default.fileExists(atPath: state.path) else {
            kinds.forEach { defaults.removeObject(forKey: "seeded.\($0)") }
            return
        }
        let lists = kinds.map { (kind: $0, names: defaults.stringArray(forKey: "seeded.\($0)")) }
        guard lists.contains(where: { $0.names != nil }) else { return }
        var text = ""
        for (kind, names) in lists {
            // File names never contain quotes or backslashes in practice; skip any that do.
            let items = (names ?? []).filter { !$0.contains("\"") && !$0.contains("\\") }.map { "\"\($0)\"" }
            text += "\(kind) = [\(items.joined(separator: ", "))]\n"
        }
        do {
            try text.write(to: state, atomically: true, encoding: .utf8)
            kinds.forEach { defaults.removeObject(forKey: "seeded.\($0)") }
        } catch {
            // Keep the keys: the next launch tries again.
        }
    }
}
