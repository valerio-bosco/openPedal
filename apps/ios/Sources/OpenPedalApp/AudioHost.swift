import AVFoundation
import Foundation
import OpenPedalCore

/// Where the processed sound goes. Persisted in UserDefaults (`audio.output`).
enum OutputMode: String, CaseIterable, Identifiable {
    /// Whatever iOS picks for the connected interface (headphone jack of the Scarlett).
    case interface
    /// The iPad's built-in speakers, with the guitar still coming from the USB interface.
    case speaker

    var id: String { rawValue }
    var label: String { self == .interface ? "Interface" : "iPad speaker" }
}

/// How "iPad speaker" is achieved. Change `AudioHost.speakerStrategy` after the device test.
enum SpeakerStrategy: String {
    /// `.playAndRecord` + `.defaultToSpeaker` + `overrideOutputAudioPort(.speaker)`, then the USB
    /// input is requested again (the override may move the input to the built-in mic).
    case override
    /// `.multiRoute`: USB input and built-in speaker at the same time, with an explicit channel map
    /// on the output unit (guitar from the USB port, sound to the speaker channels only).
    case multiRoute
}

/// What iOS actually did, for the readout in the About sheet.
struct RouteInfo: Equatable {
    var input = "-"
    var output = "-"
    var sampleRate = 0.0
    var bufferFrames = 0
    var strategy = "-"
}

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
    /// Chosen output; changing it re-applies the audio session right away.
    @Published var outputMode: OutputMode {
        didSet {
            guard outputMode != oldValue else { return }
            UserDefaults.standard.set(outputMode.rawValue, forKey: Self.outputKey)
            reconfigure()
        }
    }
    @Published private(set) var route = RouteInfo()
    /// A message for the user (feedback mute, interruption, failed re-route); nil when all is fine.
    @Published private(set) var notice: String?

    /// Strategy used for `.speaker`. Start with `.override`; if the device test shows the input stays
    /// on the built-in mic, set this to `.multiRoute`.
    static let speakerStrategy: SpeakerStrategy = .override
    /// If `.override` leaves the input on the built-in mic while a USB interface is present, try
    /// `.multiRoute` once before muting.
    static let fallbackToMultiRoute = true

    private static let outputKey = "audio.output"
    /// Largest buffer we expect from iOS (it is normally 64-512 frames).
    private static let maxFrames: UInt32 = 4096
    /// Which input channel carries the guitar on a multi-channel interface.
    /// Scarlett Solo: channel 2 (the Inst jack) = index 1. Mono devices use index 0.
    private static let guitarChannel = 1

    private var core: OpaquePointer?
    private var coreSampleRate = 48000.0
    private let engine = AVAudioEngine()
    private var inputBuffer: UnsafeMutablePointer<Float>?
    private var sink: AVAudioSinkNode?
    private var source: AVAudioSourceNode?
    private var started = false
    private var reconfiguring = false
    private var pendingReconfigure: Task<Void, Never>?
    private var activeStrategy: SpeakerStrategy?
    private var observing = false

    init() {
        let saved = UserDefaults.standard.string(forKey: Self.outputKey)
        outputMode = saved.flatMap(OutputMode.init(rawValue:)) ?? .interface
    }

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
            try applySession()

            let sampleRate = AVAudioSession.sharedInstance().sampleRate
            coreSampleRate = sampleRate
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
            observeRoute()
            refreshRoute()
            state = .running(port: openpedal_port(handle))
        } catch {
            fail("\(error.localizedDescription)")
        }
    }

    // MARK: Audio session and routing

    /// Configures the session for `outputMode` and leaves it active. Does not touch the engine.
    private func applySession() throws {
        try applySession(strategy: outputMode == .speaker ? Self.speakerStrategy : nil)
        // The override can move the input to the built-in mic: that would be speaker + mic = feedback.
        if outputMode == .speaker, activeStrategy == .override, Self.fallbackToMultiRoute,
           Self.usbInput() != nil, !Self.inputIsUSB() {
            try applySession(strategy: .multiRoute)
        }
    }

    /// `strategy == nil` means the interface output (the original behaviour).
    private func applySession(strategy: SpeakerStrategy?) throws {
        let session = AVAudioSession.sharedInstance()
        activeStrategy = strategy
        switch strategy {
        case nil:
            // .measurement turns off the system's input processing (AGC, EQ, noise reduction).
            try session.setCategory(.playAndRecord, mode: .measurement, options: [.allowBluetoothA2DP])
        case .override:
            // .measurement can force the receiver / a very low volume once the speaker is forced, so use
            // .default here. No Bluetooth option: the speaker must stay the one output.
            try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
        case .multiRoute:
            try session.setCategory(.multiRoute, mode: .default, options: [])
        }
        try session.setPreferredSampleRate(48000)
        try session.setPreferredIOBufferDuration(0.0027) // ~128 frames at 48 kHz
        try session.setActive(true)
        if strategy == .override {
            try session.overrideOutputAudioPort(.speaker)
        } else if strategy == nil {
            try session.overrideOutputAudioPort(.none)
        }
        // Prefer the USB interface for input when it is connected (again after the override).
        if let usb = Self.usbInput() { try session.setPreferredInput(usb) }
    }

    private static func usbInput() -> AVAudioSessionPortDescription? {
        AVAudioSession.sharedInstance().availableInputs?.first { $0.portType == .usbAudio }
    }

    private static func inputIsUSB() -> Bool {
        AVAudioSession.sharedInstance().currentRoute.inputs.contains { $0.portType == .usbAudio }
    }

    /// Re-applies the session and rebuilds the engine graph (the input format changes when the
    /// interface is plugged or unplugged, so the old connections would be invalid). The Rust core keeps running.
    private func reconfigure() {
        guard core != nil, !reconfiguring else { return }
        reconfiguring = true
        defer { reconfiguring = false }
        engine.stop()
        do {
            try applySession()
            if let handle = core { try buildGraph(handle: handle, sampleRate: coreSampleRate) }
            try engine.start()
            notice = nil
        } catch {
            notice = "Audio route failed: \(error.localizedDescription)"
        }
        refreshRoute()
    }

    /// Waits a moment: iOS sends several notifications for one plug/unplug.
    private func scheduleReconfigure() {
        pendingReconfigure?.cancel()
        pendingReconfigure = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            reconfigure()
        }
    }

    private func observeRoute() {
        guard !observing else { return }
        observing = true
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        center.addObserver(forName: AVAudioSession.routeChangeNotification, object: session, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
            MainActor.assumeIsolated { self?.routeChanged(reason: raw.flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))) }
        }
        center.addObserver(forName: AVAudioSession.interruptionNotification, object: session, queue: .main) { [weak self] note in
            let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
            MainActor.assumeIsolated { self?.interrupted(type: raw.flatMap(AVAudioSession.InterruptionType.init(rawValue:))) }
        }
        center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: session, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.scheduleReconfigure() }
        }
        center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.scheduleReconfigure() }
        }
    }

    private func routeChanged(reason: AVAudioSession.RouteChangeReason?) {
        switch reason {
        case .newDeviceAvailable, .oldDeviceUnavailable:
            scheduleReconfigure()
        default:
            // Includes the changes our own category/override calls cause: only refresh, never re-apply
            // (that would loop).
            if !reconfiguring { refreshRoute() }
        }
    }

    private func interrupted(type: AVAudioSession.InterruptionType?) {
        switch type {
        case .began:
            notice = "Audio interrupted"
        case .ended:
            scheduleReconfigure()
        default:
            break
        }
    }

    /// Reads what iOS did, updates the readout, and mutes the output if the guitar input fell back to
    /// the built-in mic while playing on the speaker (feedback).
    private func refreshRoute() {
        let session = AVAudioSession.sharedInstance()
        let current = session.currentRoute
        func name(_ ports: [AVAudioSessionPortDescription]) -> String {
            ports.isEmpty ? "none" : ports.map { "\($0.portName) (\($0.portType.rawValue))" }.joined(separator: ", ")
        }
        route = RouteInfo(
            input: name(current.inputs), output: name(current.outputs),
            sampleRate: session.sampleRate, bufferFrames: Int((session.ioBufferDuration * session.sampleRate).rounded()),
            strategy: activeStrategy?.rawValue ?? "interface")
        let feedback = outputMode == .speaker && current.inputs.contains { $0.portType == .builtInMic }
        engine.mainMixerNode.outputVolume = feedback ? 0 : 1
        if feedback {
            notice = "Output muted: the guitar input is the iPad microphone, which would feed back through the speaker. Plug in the interface or choose Interface."
        } else if notice?.hasPrefix("Output muted") == true {
            notice = nil
        }
    }

    /// Index of the guitar channel among the engine's input channels. `.multiRoute` numbers the channels
    /// of all input ports one after the other, so skip the ports before the USB one.
    private func guitarInputIndex() -> Int {
        guard activeStrategy == .multiRoute else { return Self.guitarChannel }
        var offset = 0
        for port in AVAudioSession.sharedInstance().currentRoute.inputs {
            if port.portType == .usbAudio { break }
            offset += port.channels?.count ?? 0
        }
        return offset + Self.guitarChannel
    }

    /// `.multiRoute` only: the output unit gets one entry per hardware output channel, the source
    /// channel to play on it or -1 for silence, so the sound reaches the built-in speaker and not the USB interface.
    private func applyOutputChannelMap() {
        guard activeStrategy == .multiRoute else { return }
        var map: [NSNumber] = []
        for port in AVAudioSession.sharedInstance().currentRoute.outputs {
            let count = port.channels?.count ?? 0
            for i in 0..<count { map.append(port.portType == .builtInSpeaker ? NSNumber(value: min(i, 1)) : -1) }
        }
        guard !map.isEmpty else { return }
        engine.outputNode.auAudioUnit.channelMap = map
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
        // A rebuild (route change): drop the old nodes first. The engine is stopped by the caller.
        if let sink { engine.detach(sink) }
        if let source { engine.detach(source) }
        let maxFrames = Self.maxFrames

        let inputFormat = engine.inputNode.outputFormat(forBus: 0)
        let channels = Int(inputFormat.channelCount)
        let pickChannel = min(guitarInputIndex(), max(channels - 1, 0))

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
        self.sink = sink
        self.source = source
        applyOutputChannelMap()
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
