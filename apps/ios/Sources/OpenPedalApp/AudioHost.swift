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

/// How "iPad speaker" is achieved. iOS gives no documented way to split a USB interface's input from
/// its output, so `applySession()` tries these in order and keeps the first that gives USB input +
/// built-in speaker output. The diagnostics log says which one worked on the device.
enum SpeakerStrategy: String, CaseIterable {
    /// `.playAndRecord` + `.defaultToSpeaker`, USB input requested first, `overrideOutputAudioPort(.speaker)`
    /// last (any later route change clears an override).
    case overrideLast
    /// Same, but the USB input is requested after the override (the override may move the input to the
    /// built-in mic).
    case override
    /// `.multiRoute`: USB input and built-in speaker at the same time, with an explicit channel map
    /// on the output unit (guitar from the USB port, sound to the speaker channels only).
    case multiRoute
}

/// Peak levels in dBFS, for the readout in the About sheet.
struct Levels: Equatable {
    var input: Float = -120
    var output: Float = -120
}

/// Appends timestamped lines to `Documents/audio-diagnostics.txt` (the previous run is kept as
/// `audio-diagnostics.prev.txt`), and to the system log. Read it from a computer with
/// `ifuse --container dev.openpedal.app <dir>`.
final class DiagLog {
    private let url: URL?
    private let start = Date()

    init() {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
        url = docs?.appendingPathComponent("audio-diagnostics.txt")
        if let url, let docs {
            let prev = docs.appendingPathComponent("audio-diagnostics.prev.txt")
            try? FileManager.default.removeItem(at: prev)
            try? FileManager.default.moveItem(at: url, to: prev)
            FileManager.default.createFile(atPath: url.path, contents: nil)
        }
        log("start \(Date())")
    }

    func log(_ line: String) {
        let text = String(format: "%7.2f ", Date().timeIntervalSince(start)) + line + "\n"
        NSLog("openPedal audio: %@", line)
        guard let url, let h = try? FileHandle(forWritingTo: url) else { return }
        h.seekToEndOfFile()
        h.write(Data(text.utf8))
        try? h.close()
    }
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
    @Published private(set) var levels = Levels()
    /// Progress of `runRoutingTest()`; nil when no test is running.
    @Published private(set) var testStatus: String?

    let diag = DiagLog()

    private static let outputKey = "audio.output"
    /// Largest buffer we expect from iOS (it is normally 64-512 frames).
    private static let maxFrames: UInt32 = 4096
    /// Which input channel carries the guitar on a multi-channel interface.
    /// Scarlett Solo: channel 2 (the Inst jack) = index 1. Mono devices use index 0.
    private static let guitarChannel = 1

    private var core: OpaquePointer?
    private var coreSampleRate = 48000.0
    /// Replaced on every graph rebuild: after a hot-plug the old engine keeps the previous input format
    /// (it read 1 channel from the 2-channel Scarlett).
    private var engine = AVAudioEngine()
    private var inputBuffer: UnsafeMutablePointer<Float>?
    private var sink: AVAudioSinkNode?
    private var source: AVAudioSourceNode?
    private var started = false
    private var reconfiguring = false
    private var pendingReconfigure: Task<Void, Never>?
    private var activeStrategy: SpeakerStrategy?
    private var observing = false
    /// Our own session calls make iOS post configuration changes too; reacting to those would
    /// restart the engine forever. Ignore them until this time.
    private var ignoreChangesUntil = Date.distantPast
    private var testing = false
    /// Peak since the last read, written by the audio callbacks: [0] guitar input, [1] output.
    private let peaks: UnsafeMutablePointer<Float> = {
        let p = UnsafeMutablePointer<Float>.allocate(capacity: 2)
        p.initialize(repeating: 0, count: 2)
        return p
    }()
    private var meterTask: Task<Void, Never>?
    /// While set, the meter task also writes the levels to the diagnostics log once a second.
    private var logLevelsUntil = Date.distantPast
    private var levelWindow = Levels()
    private var levelTicks = 0

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
        diag.log("bring up, output mode \(outputMode.rawValue)")
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
            ignoreChangesUntil = Date().addingTimeInterval(1.5)
            observeRoute()
            refreshRoute()
            startMeters()
            state = .running(port: openpedal_port(handle))
            diag.log("running")
        } catch {
            diag.log("bring up failed: \(error)")
            fail("\(error.localizedDescription)")
        }
    }

    // MARK: Meters and diagnostics

    private func startMeters() {
        guard meterTask == nil else { return }
        logLevelsUntil = Date().addingTimeInterval(20)
        meterTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                self?.readMeters()
            }
        }
    }

    private static func dB(_ x: Float) -> Float { x > 1e-6 ? 20 * log10(x) : -120 }

    private func readMeters() {
        let now = Levels(input: Self.dB(peaks[0]), output: Self.dB(peaks[1]))
        peaks[0] = 0
        peaks[1] = 0
        levels = now
        levelWindow.input = max(levelWindow.input, now.input)
        levelWindow.output = max(levelWindow.output, now.output)
        levelTicks += 1
        if levelTicks >= 4 {
            if Date() < logLevelsUntil {
                diag.log(String(format: "levels in %.0f dBFS, out %.0f dBFS", levelWindow.input, levelWindow.output))
            }
            levelWindow = Levels()
            levelTicks = 0
        }
    }

    private static func describe(_ ports: [AVAudioSessionPortDescription]?) -> String {
        guard let ports, !ports.isEmpty else { return "none" }
        return ports.map { "\($0.portName) [\($0.portType.rawValue), \($0.channels?.count ?? 0) ch]" }.joined(separator: ", ")
    }

    private func logSession(_ what: String) {
        let s = AVAudioSession.sharedInstance()
        diag.log("\(what): category \(s.category.rawValue), mode \(s.mode.rawValue), in \(Self.describe(s.currentRoute.inputs)), out \(Self.describe(s.currentRoute.outputs)), available in \(Self.describe(s.availableInputs)), \(Int(s.sampleRate)) Hz")
    }

    /// Plays every route for 5 s each while the user strums, logging what iOS did and the levels,
    /// then goes back to the chosen output. The result is in the diagnostics log.
    func runRoutingTest() {
        guard core != nil, !testing else { return }
        testing = true
        let steps: [SpeakerStrategy?] = [nil] + SpeakerStrategy.allCases
        Task { @MainActor in
            for (i, step) in steps.enumerated() {
                let name = step?.rawValue ?? "interface"
                testStatus = "Strum now: \(i + 1)/\(steps.count) \(name)"
                diag.log("TEST step \(name)")
                engine.stop()
                do {
                    try applySession(strategy: step)
                    if let handle = core { try buildGraph(handle: handle, sampleRate: coreSampleRate) }
                    try engine.start()
                } catch {
                    diag.log("TEST step \(name) failed: \(error)")
                }
                logSession("TEST \(name)")
                refreshRoute()
                logLevelsUntil = Date().addingTimeInterval(5)
                try? await Task.sleep(for: .seconds(5))
            }
            diag.log("TEST done")
            testStatus = nil
            testing = false
            reconfigure()
        }
    }

    // MARK: Audio session and routing

    /// Configures the session for `outputMode` and leaves it active. Does not touch the engine.
    private func applySession() throws {
        guard outputMode == .speaker else {
            try applySession(strategy: nil)
            logSession("session interface")
            return
        }
        // Keep the first strategy that gives USB input + speaker output. Without an interface any of
        // them lands on the built-in mic, which refreshRoute() mutes (speaker + mic = feedback).
        for (i, strategy) in SpeakerStrategy.allCases.enumerated() {
            do {
                try applySession(strategy: strategy)
            } catch {
                diag.log("session \(strategy.rawValue) failed: \(error)")
                continue
            }
            logSession("session \(strategy.rawValue)")
            let inputOK = Self.inputIsExternal() || Self.externalInput() == nil
            if inputOK && Self.outputHasSpeaker() { return }
            if i == SpeakerStrategy.allCases.count - 1 { diag.log("no strategy gave USB in + speaker out") }
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
        case .override, .overrideLast:
            // .measurement can force the receiver / a very low volume once the speaker is forced, so use
            // .default here. No Bluetooth option: the speaker must stay the one output.
            try session.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])
        case .multiRoute:
            try session.setCategory(.multiRoute, mode: .default, options: [])
        }
        try session.setPreferredSampleRate(48000)
        try session.setPreferredIOBufferDuration(0.0027) // ~128 frames at 48 kHz
        try session.setActive(true)
        switch strategy {
        case .overrideLast:
            try Self.selectExternalInput(session)
            try session.overrideOutputAudioPort(.speaker)
        case .override:
            try session.overrideOutputAudioPort(.speaker)
            try Self.selectExternalInput(session)
        case .multiRoute:
            try Self.selectExternalInput(session)
        case nil:
            try session.overrideOutputAudioPort(.none)
            // Prefer the USB interface for input when it is connected.
            try Self.selectExternalInput(session)
        }
    }

    private static func outputHasSpeaker() -> Bool {
        AVAudioSession.sharedInstance().currentRoute.outputs.contains { $0.portType == .builtInSpeaker }
    }

    /// The audio interface's input. iPadOS 26 reports the Scarlett Solo as `MicrophoneWired` (with a
    /// `Headphones` output), not `USBAudio`, so anything but the built-in mic counts.
    private static func externalInput() -> AVAudioSessionPortDescription? {
        let inputs = AVAudioSession.sharedInstance().availableInputs ?? []
        return inputs.first { $0.portType == .usbAudio } ?? inputs.first { $0.portType != .builtInMic }
    }

    private static func inputIsExternal() -> Bool {
        AVAudioSession.sharedInstance().currentRoute.inputs.contains { $0.portType != .builtInMic }
    }

    /// Selects the interface's input and asks for all its channels (the guitar is on channel 2).
    private static func selectExternalInput(_ session: AVAudioSession) throws {
        guard let port = externalInput() else { return }
        try session.setPreferredInput(port)
        try? session.setPreferredInputNumberOfChannels(min(2, session.maximumInputNumberOfChannels))
    }

    /// Re-applies the session and rebuilds the engine graph (the input format changes when the
    /// interface is plugged or unplugged, so the old connections would be invalid). The Rust core keeps running.
    private func reconfigure() {
        guard core != nil, !reconfiguring, !testing else { return }
        reconfiguring = true
        defer { reconfiguring = false }
        diag.log("reconfigure, output mode \(outputMode.rawValue)")
        engine.stop()
        do {
            try applySession()
            if let handle = core { try buildGraph(handle: handle, sampleRate: coreSampleRate) }
            try engine.start()
            notice = nil
        } catch {
            diag.log("reconfigure failed: \(error)")
            notice = "Audio route failed: \(error.localizedDescription)"
        }
        ignoreChangesUntil = Date().addingTimeInterval(1.5)
        logLevelsUntil = Date().addingTimeInterval(10)
        refreshRoute()
    }

    /// For notifications that our own session calls also trigger.
    private func externalChange(_ what: String) {
        guard !testing, Date() >= ignoreChangesUntil else { return }
        // iOS stops the engine on a real configuration change; if it still runs, there is nothing to fix.
        if what == "engine configuration" && engine.isRunning { return }
        diag.log("\(what) -> reconfigure")
        scheduleReconfigure()
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
            MainActor.assumeIsolated {
                self?.diag.log("media services reset")
                self?.scheduleReconfigure()
            }
        }
        // object nil: the engine is replaced on rebuilds, so check it is the current one.
        center.addObserver(forName: .AVAudioEngineConfigurationChange, object: nil, queue: .main) { [weak self] note in
            let sender = note.object as AnyObject?
            MainActor.assumeIsolated {
                guard let self, sender === self.engine else { return }
                self.externalChange("engine configuration")
            }
        }
    }

    private func routeChanged(reason: AVAudioSession.RouteChangeReason?) {
        diag.log("route change, reason \(reason.map { String($0.rawValue) } ?? "?")")
        switch reason {
        case .newDeviceAvailable, .oldDeviceUnavailable:
            // A plug or unplug is always real: not subject to ignoreChangesUntil.
            if !testing { scheduleReconfigure() }
        default:
            // Includes the changes our own category/override calls cause: only refresh, never re-apply
            // (that would loop).
            if !reconfiguring { refreshRoute() }
        }
    }

    private func interrupted(type: AVAudioSession.InterruptionType?) {
        switch type {
        case .began:
            diag.log("interruption began")
            notice = "Audio interrupted"
        case .ended:
            diag.log("interruption ended")
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
        let onSpeaker = testing ? Self.outputHasSpeaker() : outputMode == .speaker
        let feedback = onSpeaker && current.inputs.contains { $0.portType == .builtInMic }
        engine.mainMixerNode.outputVolume = feedback ? 0 : 1
        if feedback { diag.log("muted: built-in mic input with speaker output") }
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
            if port.portType != .builtInMic { break }
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
        // A rebuild (route change): start from a fresh engine. The old one is stopped by the caller.
        if sink != nil || source != nil {
            engine = AVAudioEngine()
            sink = nil
            source = nil
        }
        let maxFrames = Self.maxFrames

        let inputFormat = engine.inputNode.outputFormat(forBus: 0)
        let channels = Int(inputFormat.channelCount)
        let pickChannel = min(guitarInputIndex(), max(channels - 1, 0))
        let peaks = self.peaks
        diag.log("graph: hardware input \(engine.inputNode.inputFormat(forBus: 0).channelCount) ch, input \(channels) ch at \(Int(inputFormat.sampleRate)) Hz, guitar channel \(pickChannel), output \(engine.outputNode.outputFormat(forBus: 0).channelCount) ch")

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
            var peak = peaks[0]
            for i in 0..<Int(frames) { peak = max(peak, abs(input[i])) }
            peaks[0] = peak
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
            var peak = peaks[1]
            for i in 0..<Int(frameCount) { peak = max(peak, abs(left[i])) }
            peaks[1] = peak
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
