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
    private var coreSampleRate = 48000.0
    /// Replaced on every rebuild: after a hot-plug the old engine keeps the previous input format
    /// (it read 1 channel, the mic one, from the 2-channel Scarlett).
    private var engine = AVAudioEngine()
    private var inputBuffer: UnsafeMutablePointer<Float>?
    private var started = false
    /// Slot read by the audio thread: the active `DIRecorder` (unretained) or nil. Allocated once.
    private let recorderSlot: UnsafeMutablePointer<UnsafeMutableRawPointer?> = {
        let p = UnsafeMutablePointer<UnsafeMutableRawPointer?>.allocate(capacity: 1)
        p.initialize(to: nil)
        return p
    }()
    /// Owns the recorder while it is published in `recorderSlot` (see Recorder.swift).
    var activeRecorder: DIRecorder? {
        didSet {
            recorderSlot.pointee = activeRecorder.map { Unmanaged.passUnretained($0).toOpaque() }
        }
    }
    var isRunning: Bool { if case .running = state { return true } else { return false } }
    var currentSampleRate: Double { coreSampleRate }
    private var pendingRebuild: Task<Void, Never>?
    /// Our own session calls make iOS post configuration changes too; reacting to those would
    /// restart the engine forever. Ignore them until this time.
    private var ignoreChangesUntil = Date.distantPast

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
            try Self.applySession()
            let sampleRate = AVAudioSession.sharedInstance().sampleRate
            coreSampleRate = sampleRate
            let dataDir = try Self.prepareDataDir()
            let factory = Self.factoryDir()
            // Demucs weights bundled by build-core.sh (separation builds only); selects the native Tone Lab backend.
            if let weights = Self.bundledWeights() { weights.withCString { openpedal_set_demucs_weights($0) } }
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
            ignoreChangesUntil = Date().addingTimeInterval(1.5)
            observeRoute()
            state = .running(port: openpedal_port(handle))
        } catch {
            fail("\(error.localizedDescription)")
        }
    }

    private static func applySession() throws {
        let session = AVAudioSession.sharedInstance()
        // .measurement turns off the system's input processing (AGC, EQ, noise reduction).
        try session.setCategory(.playAndRecord, mode: .measurement, options: [.allowBluetoothA2DP])
        try session.setPreferredSampleRate(48000)
        try session.setPreferredIOBufferDuration(0.0027) // ~128 frames at 48 kHz
        try session.setActive(true)
        // Prefer the interface for input when it is connected, with all its channels (the guitar is
        // on channel 2). iPadOS 26 reports the Scarlett Solo as MicrophoneWired, not USBAudio, so
        // anything but the built-in mic counts.
        let inputs = session.availableInputs ?? []
        if let port = inputs.first(where: { $0.portType == .usbAudio }) ?? inputs.first(where: { $0.portType != .builtInMic }) {
            try session.setPreferredInput(port)
            try? session.setPreferredInputNumberOfChannels(min(2, session.maximumInputNumberOfChannels))
        }
    }

    // MARK: Plugging and unplugging the interface

    private func observeRoute() {
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        center.addObserver(forName: AVAudioSession.routeChangeNotification, object: session, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
            let reason = raw.flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
            guard reason == .newDeviceAvailable || reason == .oldDeviceUnavailable else { return }
            MainActor.assumeIsolated { self?.scheduleRebuild() }
        }
        center.addObserver(forName: AVAudioSession.interruptionNotification, object: session, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
            guard raw == AVAudioSession.InterruptionType.ended.rawValue else { return }
            MainActor.assumeIsolated { self?.scheduleRebuild() }
        }
        center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: session, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.scheduleRebuild() }
        }
        // object nil: the engine is replaced on rebuilds, so check it is the current one.
        center.addObserver(forName: .AVAudioEngineConfigurationChange, object: nil, queue: .main) { [weak self] note in
            let sender = note.object as AnyObject?
            MainActor.assumeIsolated {
                // iOS stops the engine on a real change; skip the ones our own rebuild caused.
                guard let self, sender === self.engine, !self.engine.isRunning,
                      Date() >= self.ignoreChangesUntil else { return }
                self.scheduleRebuild()
            }
        }
    }

    /// Waits a moment: iOS sends several notifications for one plug/unplug.
    private func scheduleRebuild() {
        pendingRebuild?.cancel()
        pendingRebuild = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            rebuild()
        }
    }

    /// Re-applies the session and starts a fresh engine for the new input format. The Rust core keeps running.
    private func rebuild() {
        guard let handle = core else { return }
        engine.stop()
        engine = AVAudioEngine()
        do {
            try Self.applySession()
            try buildGraph(handle: handle, sampleRate: coreSampleRate)
            try engine.start()
        } catch {
            NSLog("openPedal: audio rebuild failed: %@", "\(error)")
        }
        ignoreChangesUntil = Date().addingTimeInterval(1.5)
    }

    private func buildGraph(handle: OpaquePointer, sampleRate: Double) throws {
        // Preallocated once, used from the audio thread: no allocation in the callbacks.
        let capacity = Int(Self.maxFrames)
        let input: UnsafeMutablePointer<Float>
        if let existing = inputBuffer {
            input = existing
        } else {
            input = UnsafeMutablePointer<Float>.allocate(capacity: capacity)
            input.initialize(repeating: 0, count: capacity)
            inputBuffer = input
        }
        let channel = Self.guitarChannel
        let slot = recorderSlot
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
            // DI recording: plain copy into the recorder's preallocated buffer, if one is armed.
            if let raw = slot.pointee {
                Unmanaged<DIRecorder>.fromOpaque(raw)._withUnsafeGuaranteedRef { $0.push(input, frames: Int(frames)) }
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

    private static func bundledWeights() -> String? {
        guard let url = Bundle.module.url(forResource: "ggml-model-htdemucs-6s-f16", withExtension: "bin", subdirectory: "models"),
              FileManager.default.fileExists(atPath: url.path) else { return nil }
        return url.path
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
