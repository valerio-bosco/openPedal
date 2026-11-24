import SwiftUI

/// One picked part: its input (DI), Record, Fit, the result in plain words, A/B player and Save.
struct PartCard: View {
    let store: Store
    let job: TonelabJobInfo
    let part: TonelabPart
    @Environment(\.audioHostRecording) private var recorder
    @State private var name = ""
    @State private var ab = ABPlayerModel()

    private var model: ToneLabModel { store.toneLab }
    private var result: ToneLabFitResult? { model.fits[part.id] }
    private var recording: Bool { model.recordingPart == part.id }
    private var seconds: Int { Int((part.end - part.start).rounded(.up)) }
    private var fitting: Bool { model.busy && model.progress?.stage == .fit }
    private var saved: Bool { model.savedNames[part.id].map { store.presets.contains($0) } ?? false }

    private var diText: String {
        if part.hasUserDi { return "your recording" }
        if part.hasSynthDi { return "the synthetic input" }
        return "nothing yet (the synthetic input is still being built)"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text(part.label).font(.title3.weight(.semibold))
                Text("\(tlClock(part.start)) to \(tlClock(part.end)), \(part.channel.rawValue), \(part.notes) notes")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Label { Text("\(Text("The fit will use ").foregroundStyle(.secondary))\(Text(diText).bold())") } icon: { Image(systemName: "guitars") }
                .font(.callout)
            Text("A recording of you playing this part (\(seconds) s, after a 2 s countdown) makes the preset fit your own guitar. Play it dry, on the guitar input. Without one, a synthetic input built from the detected notes is used.")
                .font(.footnote).foregroundStyle(.secondary)
            HStack(spacing: 12) {
                Button { model.record(part: part, recorder: recorder) } label: {
                    Label(part.hasUserDi ? "Record again" : "Record my take", systemImage: "record.circle")
                }
                .buttonStyle(.glass).controlSize(.large)
                .disabled(model.busy || model.recordingPart != nil)
                Button { model.fit(part: part.id) } label: {
                    Label(result == nil ? "Fit this part" : "Fit again", systemImage: "wand.and.stars")
                }
                .buttonStyle(.glassProminent).controlSize(.large)
                .disabled(model.busy || model.recordingPart != nil || (!part.hasUserDi && !part.hasSynthDi))
            }
            if recording {
                Label(model.recordStatus, systemImage: "waveform").font(.callout).foregroundStyle(.orange)
            }
            if model.busy, model.progress?.stage == .fit || model.progress?.stage == .record || model.progress?.stage == .synth {
                VStack(alignment: .leading, spacing: 8) {
                    if let p = model.progress {
                        if let f = p.fraction { ProgressView(value: f) } else { ProgressView() }
                        Text(p.message).font(.callout).foregroundStyle(.secondary)
                    }
                    Button("Cancel", role: .cancel) { model.cancel() }.buttonStyle(.glass)
                }
            }
            if let r = result { resultCard(r) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(18)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 26))
        .onAppear { if name.isEmpty { name = "\(job.title) \(part.label)".replacingOccurrences(of: "/", with: "-").trimmingCharacters(in: .whitespaces) } }
        .onDisappear { ab.stop() }
    }

    private func resultCard(_ r: ToneLabFitResult) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider()
            HStack(alignment: .firstTextBaseline) {
                Text("Result:").font(.headline)
                Text(r.fit.template).font(.headline).foregroundStyle(Color(hex: 0xff8a3d))
            }
            numbers(r.fit)
            if !r.templates.isEmpty { table(r) }
            abPlayer
            HStack {
                TextField("Preset name", text: $name).textFieldStyle(.roundedBorder)
                Button { model.savePreset(part: part.id, name: name.trimmingCharacters(in: .whitespaces)) } label: {
                    Label("Save as preset", systemImage: "square.and.arrow.down")
                }
                .buttonStyle(.glassProminent).controlSize(.large)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if saved, let n = model.savedNames[part.id] {
                HStack {
                    Label("Saved. It is in the preset menu.", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                    Spacer()
                    Button("Load it") { store.loadPreset(n) }.buttonStyle(.glass)
                }
                .font(.callout)
            }
        }
    }

    private func numbers(_ fit: TonelabFit) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            line(tlDb(fit.errorDb), "spectral distance to the song. It measures how far the tone's colour (bass, mids, treble) is from the song's. Lower is better; under 3 dB is very close.")
            if let h = fit.holdoutErrorDb {
                line(tlDb(h), "on audio the fit never saw (held out). If this is much higher than the number above, the preset may only suit the input it was fitted on.")
            }
            if let v = fit.validateErrorDb {
                line(tlDb(v), "on a real recording of your guitar. This is the honest number: how the preset sounds with a real input.")
            }
        }
    }

    private func line(_ number: String, _ text: String) -> some View {
        Text("\(Text(number).bold()) \(Text(text).foregroundStyle(.secondary))").font(.callout)
    }

    private func table(_ r: ToneLabFitResult) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 6) {
            GridRow {
                Text("Template"); Text("Error"); Text("Renders"); Text("Stage")
            }
            .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            ForEach(r.templates, id: \.template) { t in
                GridRow {
                    Text(t.template)
                    Text(tlDb(t.errorDb)).monospacedDigit()
                    Text("\(t.renders)").monospacedDigit()
                    Text(t.stage)
                }
                .font(.callout)
                .foregroundStyle(t.template == r.fit.template ? Color(hex: 0xff8a3d) : .primary)
            }
        }
    }

    private var abPlayer: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Listen: the song's part against your preset").font(.subheadline.weight(.semibold))
            HStack(spacing: 12) {
                Button { ab.toggle() } label: { Image(systemName: ab.playing ? "pause.fill" : "play.fill").frame(width: 30, height: 30) }
                    .buttonStyle(.glass).buttonBorderShape(.circle)
                    .disabled(!ab.ready)
                Picker("Source", selection: Binding(get: { ab.side }, set: { ab.choose($0) })) {
                    Text("Song (target)").tag(ABPlayerModel.Side.target)
                    Text("Preset (fit)").tag(ABPlayerModel.Side.result)
                }
                .pickerStyle(.segmented).frame(maxWidth: 320)
                Text("\(tlClock(ab.position)) / \(tlClock(ab.duration))").font(.footnote.monospacedDigit()).foregroundStyle(.secondary)
            }
            ScrubBar(value: ab.position, total: ab.duration) { ab.seek($0) }
            if !ab.ready { Text("The audio files are not ready yet.").font(.footnote).foregroundStyle(.secondary) }
        }
        .task(id: result?.fit) {
            ab.load(target: ToneLabPaths.target(job.id, part.id), result: ToneLabPaths.render(job.id, part.id))
        }
    }
}
