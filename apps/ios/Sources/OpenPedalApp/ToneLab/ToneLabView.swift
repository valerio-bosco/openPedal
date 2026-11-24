import SwiftUI
import UniformTypeIdentifiers

/// Tone Lab: a song becomes a preset (docs/PLAN.md §14, §15). Full-screen glass cover.
struct ToneLabView: View {
    let store: Store
    @Environment(\.dismiss) private var dismiss
    @Environment(\.audioHostRecording) private var recorder
    @State private var picking = false
    @State private var peaks: [Float] = []
    @State private var peakDuration: Double = 0
    @State private var selection: ClosedRange<Double>?
    @State private var channel: TonelabChannel = .mid
    @State private var label = ""
    @State private var player = RegionPlayer()

    private var model: ToneLabModel { store.toneLab }

    private static let audioTypes: [UTType] = [.mp3, .mpeg4Audio, .wav, .aiff, .audio]

    var body: some View {
        ZStack(alignment: .top) {
            Ambient(tint: Color(hex: 0xff8a3d))
            VStack(spacing: 14) {
                topBar
                ScrollView {
                    VStack(spacing: 16) {
                        if let reason = model.reason, !model.available {
                            card { Label("Tone Lab is not available: \(reason)", systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) }
                        }
                        sourceCard
                        if let job = model.job {
                            songCard(job)
                            if !job.parts.isEmpty { partsSection(job) }
                        }
                    }
                    .frame(maxWidth: 900)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 20).padding(.bottom, 30)
                }
                .scrollDismissesKeyboard(.interactively)
            }
            if let text = model.error ?? store.banner {
                Label(text, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout.weight(.semibold))
                    .padding(.horizontal, 18).padding(.vertical, 10)
                    .glassEffect(.regular.tint(.red.opacity(0.35)), in: .capsule)
                    .padding(.top, 64)
                    .onTapGesture { model.error = nil }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear { model.refresh() }
        .onDisappear { player.stop() }
        .task(id: model.job?.id) { await loadWave() }
        .fileImporter(isPresented: $picking, allowedContentTypes: Self.audioTypes) { result in
            switch result {
            case .success(let url): model.importFile(url)
            case .failure(let e): model.error = e.localizedDescription
            }
        }
    }

    // MARK: pieces

    private var topBar: some View {
        HStack {
            Text("Tone Lab").font(.title2.weight(.bold))
            Spacer()
            Button("Done") { dismiss() }.buttonStyle(.glassProminent).controlSize(.large)
        }
        .padding(.horizontal, 20).padding(.top, 8)
    }

    private func card<C: View>(@ViewBuilder _ content: () -> C) -> some View {
        content()
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(18)
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 26))
    }

    private var sourceCard: some View {
        card {
            VStack(alignment: .leading, spacing: 12) {
                Text("1. Song").font(.headline)
                Text("Pick a song from Files (mp3, m4a, wav, aiff), or a Voice Memo exported to Files.")
                    .font(.callout).foregroundStyle(.secondary)
                HStack {
                    Button { picking = true } label: { Label("Import audio", systemImage: "square.and.arrow.down") }
                        .buttonStyle(.glassProminent).controlSize(.large)
                        .disabled(model.busy || model.importing != nil || !model.available)
                    if !model.jobs.isEmpty {
                        Menu {
                            ForEach(model.jobs, id: \.id) { j in
                                Button("\(j.title) (\(tlClock(j.durationS)))") { model.open(j.id) }
                            }
                        } label: { Label("Earlier songs", systemImage: "clock.arrow.circlepath") }
                            .buttonStyle(.glass).controlSize(.large)
                            .disabled(model.busy)
                    }
                }
                if let f = model.importing {
                    ProgressView(value: f) { Text("Reading the file…").font(.callout) }
                    Button("Cancel", role: .cancel) { model.cancelImport() }.buttonStyle(.glass)
                } else if model.busy, model.job == nil || model.progress?.stage == .fetch {
                    progressView
                }
            }
        }
    }

    private var progressView: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let p = model.progress {
                if let f = p.fraction { ProgressView(value: f) } else { ProgressView().frame(maxWidth: .infinity, alignment: .leading) }
                Text(stageText(p)).font(.callout).foregroundStyle(.secondary)
            } else {
                ProgressView()
            }
            Button("Cancel", role: .cancel) { model.cancel() }.buttonStyle(.glass)
        }
    }

    private func stageText(_ p: ToneLabProgress) -> String {
        let head: String
        switch p.stage {
        case .fetch: head = "Reading"
        case .separate: head = "Separating the guitar"
        case .analyse: head = "Analysing"
        case .parts: head = "Cutting the parts"
        case .synth: head = "Building the synthetic input"
        case .record: head = "Recording"
        case .fit: head = "Fitting"
        case .cancelled: head = "Cancelled"
        }
        return p.message.isEmpty ? head : "\(head): \(p.message)"
    }

    private func songCard(_ job: TonelabJobInfo) -> some View {
        let dur = job.durationS > 0 ? job.durationS : peakDuration
        return card {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    Text("2. Mark the part").font(.headline)
                    Spacer()
                    Text(job.title).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                }
                HStack(spacing: 14) {
                    Text(tlClock(dur))
                    if let t = job.tempoBpm { Text("\(Int(t.rounded())) BPM") }
                    if let k = job.key { Text(k) }
                }
                .font(.footnote).foregroundStyle(.secondary)

                if peaks.isEmpty || dur <= 0 {
                    ProgressView().frame(maxWidth: .infinity, minHeight: 120)
                } else {
                    WaveformView(peaks: peaks, duration: dur, sections: job.sections, selection: $selection,
                                 playhead: player.playing ? player.position : nil)
                        .frame(height: job.sections.isEmpty ? 150 : 162)
                }
                if !job.sections.isEmpty { legend }
                Text("Drag on the waveform to mark the part to copy; drag the orange edges to adjust. Keep it short (10 to 25 s): one riff.")
                    .font(.footnote).foregroundStyle(.secondary)

                HStack(spacing: 12) {
                    Button {
                        if player.playing { player.stop() } else if let s = selection {
                            player.play(ToneLabPaths.mix(job.id), from: s.lowerBound, to: s.upperBound)
                        }
                    } label: { Label(player.playing ? "Stop" : "Play selection", systemImage: player.playing ? "stop.fill" : "play.fill") }
                        .buttonStyle(.glass).disabled(selection == nil)
                    if let s = selection {
                        Text("\(tlClock(s.lowerBound)) to \(tlClock(s.upperBound))  (\(Int((s.upperBound - s.lowerBound).rounded())) s)")
                            .font(.callout.monospacedDigit())
                    } else {
                        Text("Nothing selected").font(.callout).foregroundStyle(.secondary)
                    }
                }

                Divider()
                Text("3. Channel and name").font(.headline)
                Picker("Channel", selection: $channel) {
                    Text("Middle").tag(TonelabChannel.mid)
                    Text("Left").tag(TonelabChannel.left)
                    Text("Right").tag(TonelabChannel.right)
                }
                .pickerStyle(.segmented)
                Text("Which side of the stereo mix the guitar is on. Middle for one guitar in the centre; left or right to isolate one of a pair.")
                    .font(.footnote).foregroundStyle(.secondary)
                TextField("Name of the part (riff, solo…)", text: $label)
                    .textFieldStyle(.roundedBorder)

                if model.busy, model.progress?.stage != .fetch, model.recordingPart == nil, model.progress?.stage != .fit {
                    progressView
                } else {
                    Button { separate() } label: { Label("Separate & pick", systemImage: "scissors") }
                        .buttonStyle(.glassProminent).controlSize(.large)
                        .disabled(selection == nil || model.busy)
                    Label("Separating the guitar takes about a minute on the iPad. Keep the app open.", systemImage: "hourglass")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var legend: some View {
        HStack(spacing: 14) {
            ForEach(["centre", "double", "wide"], id: \.self) { k in
                HStack(spacing: 5) {
                    Circle().fill(tlKindColor[k] ?? .gray).frame(width: 8, height: 8)
                    Text((tlKindText[k] ?? k).components(separatedBy: ":").first ?? k).font(.caption)
                }
            }
        }
        .accessibilityHidden(true)
    }

    private func separate() {
        guard let s = selection else { return }
        player.stop()
        let name = label.trimmingCharacters(in: .whitespaces)
        model.pick(start: s.lowerBound, end: s.upperBound, channel: channel, label: name.isEmpty ? "riff" : name)
    }

    private func partsSection(_ job: TonelabJobInfo) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("4. Parts").font(.headline).padding(.leading, 6)
            ForEach(job.parts, id: \.id) { part in
                PartCard(store: store, job: job, part: part)
            }
        }
    }

    private func loadWave() async {
        peaks = []; selection = nil; peakDuration = 0
        guard let id = model.job?.id else { return }
        if let r = await ToneLabImporter.peaks(of: ToneLabPaths.mix(id)) {
            peaks = r.peaks; peakDuration = r.duration
        }
    }
}
