import AVFoundation
import Foundation
import Observation

struct ToneLabProgress: Equatable {
    var job: String
    var stage: TonelabStage
    var message: String
    var fraction: Double?
}

struct ToneLabFitResult {
    var fit: TonelabFit
    var templates: [TonelabTemplateResult]
}

/// State of the Tone Lab screen. The Store routes `tonelab_*` messages here and gives it `send`.
@MainActor @Observable
final class ToneLabModel {
    var send: (ClientMsg) -> Void = { _ in }

    private(set) var capsKnown = false
    private(set) var available = true
    private(set) var reason: String?
    private(set) var jobs: [TonelabJobSummary] = []
    private(set) var job: TonelabJobInfo?
    private(set) var progress: ToneLabProgress?
    private(set) var fits: [String: ToneLabFitResult] = [:]
    /// A step is running in the core (start, pick, record, fit).
    private(set) var busy = false
    /// Decoding the imported file: 0...1.
    private(set) var importing: Double?
    var error: String?
    /// Part currently being recorded (and a short status).
    private(set) var recordingPart: String?
    private(set) var recordStatus = ""
    /// Preset names this screen asked to save.
    private(set) var savedNames: [String: String] = [:]

    private var importTask: Task<Void, Never>?

    func refresh() {
        send(.tonelabCaps)
        send(.tonelabJobs)
    }

    func open(_ id: String) {
        fits = [:]
        send(.tonelabOpen(id))
    }

    func closeJob() { job = nil; fits = [:]; progress = nil }

    // MARK: server messages

    func handle(_ msg: ServerMsg) {
        switch msg {
        case .tonelabCaps(let a, _, _, let r):
            capsKnown = true; available = a; reason = r
        case .tonelabJobs(let list):
            jobs = list.sorted { $0.created > $1.created }
        case .tonelabProgress(let j, let stage, let message, let fraction):
            if stage == .cancelled {
                busy = false; progress = nil
            } else {
                busy = true
                progress = ToneLabProgress(job: j, stage: stage, message: message, fraction: fraction)
            }
        case .tonelabJob(let info):
            if job?.id != info.id { fits = [:] }
            job = info
            busy = false; progress = nil
            for p in info.parts { if let f = p.fit, fits[p.id] == nil { fits[p.id] = ToneLabFitResult(fit: f, templates: []) } }
            send(.tonelabJobs)
        case .tonelabFitResult(_, let part, let fit, let templates):
            fits[part] = ToneLabFitResult(fit: fit, templates: templates)
            busy = false; progress = nil
        default:
            break
        }
    }

    /// The core answered with `error`: whatever was running is over.
    func failed(_ message: String) {
        busy = false; progress = nil; importing = nil
        if recordingPart == nil { error = message }
    }

    // MARK: actions

    func importFile(_ url: URL) {
        guard importTask == nil, !busy else { return }
        error = nil
        importing = 0
        importTask = Task { [weak self] in
            do {
                let r = try await ToneLabImporter.importAudio(from: url) { f in
                    Task { @MainActor in self?.importing = f }
                }
                guard let self else { return }
                self.importing = nil
                self.fits = [:]
                self.busy = true
                self.progress = ToneLabProgress(job: r.id, stage: .fetch, message: "Reading the song", fraction: nil)
                self.send(.tonelabStart(r.mix.path))
            } catch {
                self?.importing = nil
                if !(error is CancellationError) { self?.error = error.localizedDescription }
            }
            self?.importTask = nil
        }
    }

    func cancelImport() { importTask?.cancel() }

    func pick(start: Double, end: Double, channel: TonelabChannel, label: String) {
        guard let job else { return }
        error = nil
        busy = true
        progress = ToneLabProgress(job: job.id, stage: .separate, message: "Starting separation", fraction: nil)
        send(.tonelabPick(job: job.id, start: start, end: end, channel: channel, label: label))
    }

    func cancel() {
        let id = progress?.job ?? job?.id
        if let id { send(.tonelabCancel(id)) }
        cancelImport()
    }

    func fit(part: String) {
        guard let job else { return }
        error = nil
        busy = true
        progress = ToneLabProgress(job: job.id, stage: .fit, message: "Fitting", fraction: nil)
        send(.tonelabFit(job: job.id, part: part))
    }

    func savePreset(part: String, name: String) {
        guard let job else { return }
        savedNames[part] = name
        send(.tonelabSavePreset(job: job.id, part: part, name: name))
    }

    func record(part: TonelabPart, recorder: (any AudioHostRecording)?) {
        guard let job, recordingPart == nil, !busy else { return }
        error = nil
        let seconds = (part.end - part.start).rounded(.up)
        let countIn = 2.0
        let url = ToneLabPaths.userDI(job.id, part.id)
        recordingPart = part.id
        recordStatus = "Get ready: recording starts in \(Int(countIn)) s, then play the part for \(Int(seconds)) s."
        let jobID = job.id, partID = part.id
        Task { [weak self] in
            do {
                guard let recorder else { throw RecordingUnavailable() }
                try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try await recorder.recordDI(to: url, seconds: seconds, countIn: countIn)
                guard let self else { return }
                self.recordingPart = nil
                self.busy = true
                self.send(.tonelabRecordDi(job: jobID, part: partID, seconds: seconds))
            } catch {
                self?.recordingPart = nil
                self?.error = error.localizedDescription
            }
        }
    }
}
