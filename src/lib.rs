use nih_plug::prelude::*;
use std::sync::Arc;

mod envelope;

use crate::envelope::AREnvelope;

const BLOCK_SIZE: usize = 1050; // 1050 samples = 42Hz wave period at 44.1k

struct AdditiveEngine {
    working_harmonic_amplitudes_l: [f32; 512],
    working_harmonic_amplitudes_r: [f32; 512],
    harmonic_amplitudes_l: [f32; 512],
    harmonic_amplitudes_r: [f32; 512],
    harmonic_phases: [f32; 512],
    block_progress: usize,
    prev_harmonic: usize,
    harmonic_sample_count: usize,
}

impl Default for AdditiveEngine {
    fn default() -> Self {
        Self {
            working_harmonic_amplitudes_l: [0.0; 512],
            working_harmonic_amplitudes_r: [0.0; 512],
            harmonic_amplitudes_l: [0.0; 512],
            harmonic_amplitudes_r: [0.0; 512],
            harmonic_phases: [0.0; 512],
            block_progress: 0,
            prev_harmonic: 0,
            harmonic_sample_count: 0,
        }
    }
}

struct AthenicDemodulator {
    params: Arc<AthenicDemodulatorParams>,
    engine: AdditiveEngine,
    sample_rate: f32,
    notes_on: usize,
    current_midi_note: u8,
    bend_amount: f32,
    envelope: AREnvelope,
    envelope_values: Vec<f32>,
}

#[derive(Params)]
struct AthenicDemodulatorParams {
    #[id = "floor"]
    floor: FloatParam,
    #[id = "ceiling"]
    ceiling: FloatParam,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "attack_ms"]
    attack_ms: FloatParam,
    #[id = "release_ms"]
    release_ms: FloatParam,
    #[id = "partial_count"]
    partial_count: IntParam,
    #[id = "partial_offset"]
    partial_offset: IntParam,
}

impl Default for AthenicDemodulator {
    fn default() -> Self {
        let mut envelope_values = Vec::new();
        envelope_values.resize_with(4096, || 0.0);

        Self {
            params: Arc::new(AthenicDemodulatorParams::default()),
            engine: AdditiveEngine::default(),
            sample_rate: 44100.0,
            notes_on: 0,
            current_midi_note: 0,
            bend_amount: 0.0,
            envelope: AREnvelope::default(),
            envelope_values,
        }
    }
}

impl Default for AthenicDemodulatorParams {
    fn default() -> Self {
        Self {
            floor: FloatParam::new(
                "floor",
                0.0,
                FloatRange::Linear {
                    min: -2.0,
                    max: 2.0,
                },
            )
            .with_step_size(1.0 / 32.0),
            ceiling: FloatParam::new(
                "ceiling",
                2.0,
                FloatRange::Linear {
                    min: -2.0,
                    max: 2.0,
                },
            )
            .with_step_size(1.0 / 32.0),
            bias: FloatParam::new(
                "bias",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_step_size(1.0 / 64.0),

            attack_ms: FloatParam::new(
                "attack",
                0.5,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.001),
            release_ms: FloatParam::new(
                "release",
                0.5,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-2.5),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.001),

            partial_count: IntParam::new(
                "partial count",
                500,
                IntRange::Linear { min: 1, max: 512 },
            ),
            partial_offset: IntParam::new(
                "partial offset",
                0,
                IntRange::Linear { min: 0, max: 512 },
            ),
        }
    }
}

impl Plugin for AthenicDemodulator {
    const NAME: &'static str = "athenic demodulator";
    const VENDOR: &'static str = "charlotte athena som";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "charlotte@som.codes";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),

        aux_input_ports: &[],
        aux_output_ports: &[],

        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        context.set_latency_samples(BLOCK_SIZE as u32);

        true
    }

    fn reset(&mut self) {
        self.envelope.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        let buf = buffer.as_slice();

        let mut note_event = context.next_event();

        self.envelope
            .set_attack_time(self.sample_rate, self.params.attack_ms.value());
        self.envelope
            .set_release_time(self.sample_rate, self.params.release_ms.value());
        self.envelope
            .next_block(&mut self.envelope_values, num_samples);

        let num_partials = self.params.partial_count.value() as usize;
        let partial_offset = self.params.partial_offset.value() as usize;

        for sample_idx in 0..num_samples {
            'events: loop {
                match note_event {
                    Some(event) if (event.timing() as usize) == sample_idx => {
                        // if the event is for the current sample, we should process it:
                        match event {
                            NoteEvent::NoteOn { note, .. } => {
                                if self.notes_on == 0 {
                                    self.envelope.reset();

                                    self.engine.block_progress = 0;
                                    self.engine.prev_harmonic = 0;

                                    // TODO: other phase modes
                                    for (i, phi) in
                                        self.engine.harmonic_phases.iter_mut().enumerate()
                                    {
                                        *phi = 512.0 / (i + 1) as f32;
                                    }
                                }

                                self.notes_on += 1;
                                self.current_midi_note = note;
                            }
                            NoteEvent::NoteOff { .. } => {
                                self.notes_on = self.notes_on.saturating_sub(1);
                                if self.notes_on == 0 {
                                    self.envelope.start_release();
                                }
                            }
                            NoteEvent::MidiPitchBend { value, .. } => {
                                self.bend_amount = value;
                            }
                            _ => {}
                        }

                        // then get the next one so we don't loop forever
                        note_event = context.next_event();
                    }
                    Some(event) if (event.timing() as usize) < sample_idx => {
                        // if the event already passed, try the next one:
                        note_event = context.next_event();
                    }
                    _ => {
                        // if the event is after the current sample, just hold on to it:
                        break 'events;
                    }
                }
            }

            if self.engine.block_progress < BLOCK_SIZE {
                let next_harmonic: usize = f32::floor(f32::exp(
                    f32::ln(num_partials as f32) * (self.engine.block_progress as f32)
                        / BLOCK_SIZE as f32,
                )) as usize
                    + 1
                    + partial_offset;

                let mut amplitude_l = buf[0][sample_idx];
                amplitude_l += self.params.bias.value();
                if amplitude_l > self.params.ceiling.value() {
                    amplitude_l = 0.0;
                }
                if amplitude_l < self.params.floor.value() {
                    amplitude_l = 0.0;
                }

                let mut amplitude_r = buf[1][sample_idx];
                amplitude_r += self.params.bias.value();
                if amplitude_r > self.params.ceiling.value() {
                    amplitude_r = 0.0;
                }
                if amplitude_r < self.params.floor.value() {
                    amplitude_r = 0.0;
                }

                for harmonic in self.engine.prev_harmonic + 1..=next_harmonic {
                    if harmonic >= 512 {
                        break;
                    }
                    if harmonic != self.engine.prev_harmonic {
                        self.engine.working_harmonic_amplitudes_l[harmonic - 1] =
                            amplitude_l * amplitude_l * amplitude_l.signum();
                        self.engine.working_harmonic_amplitudes_r[harmonic - 1] =
                            amplitude_r * amplitude_r * amplitude_r.signum();
                        self.engine.harmonic_sample_count = 1;
                    } else {
                        // moving average
                        self.engine.harmonic_sample_count += 1;
                        self.engine.working_harmonic_amplitudes_l[harmonic - 1] =
                            (self.engine.working_harmonic_amplitudes_l[harmonic - 1]
                                * (self.engine.harmonic_sample_count as f32 - 1.0)
                                + amplitude_l * amplitude_l * amplitude_l.signum())
                                / (self.engine.harmonic_sample_count as f32);
                        self.engine.working_harmonic_amplitudes_r[harmonic - 1] =
                            (self.engine.working_harmonic_amplitudes_r[harmonic - 1]
                                * (self.engine.harmonic_sample_count as f32 - 1.0)
                                + amplitude_r * amplitude_r * amplitude_r.signum())
                                / (self.engine.harmonic_sample_count as f32);
                    }

                    self.engine.prev_harmonic = harmonic;
                }
            }

            self.engine.block_progress += 1;

            buf[0][sample_idx] = 0.0;
            buf[1][sample_idx] = 0.0;

            if self.engine.block_progress >= BLOCK_SIZE {
                self.engine.block_progress = 0;
                self.engine
                    .harmonic_amplitudes_l
                    .copy_from_slice(&self.engine.working_harmonic_amplitudes_l);
                self.engine
                    .harmonic_amplitudes_r
                    .copy_from_slice(&self.engine.working_harmonic_amplitudes_r);
                self.engine.prev_harmonic = 0;
            }

            if self.notes_on > 0 || self.envelope.is_releasing() {
                let fundamental = util::f32_midi_note_to_freq(
                    self.current_midi_note as f32
                        + (self.bend_amount.clamp(0.0, 1.0) * 2.0 - 1.0) * 12.0, // 12.0 = BEND_EXTENTS
                );

                for harmonic_idx in 0..num_partials {
                    let frequency = fundamental * (1.0 + harmonic_idx as f32);
                    let step = frequency / self.sample_rate;

                    let gain_l = self.engine.harmonic_amplitudes_l[harmonic_idx]
                        * self.envelope_values[sample_idx];
                    let gain_r = self.engine.harmonic_amplitudes_r[harmonic_idx]
                        * self.envelope_values[sample_idx];
                    let phase = self.engine.harmonic_phases[harmonic_idx];

                    self.engine.harmonic_phases[harmonic_idx] = phase + step;

                    if frequency < self.sample_rate / 2.0 {
                        let (s, c) = f32::sin_cos(phase * std::f32::consts::TAU);
                        let sample_l = (s + c) * gain_l;
                        let sample_r = (s - c) * gain_r;

                        buf[0][sample_idx] += sample_l;
                        buf[1][sample_idx] += sample_r;
                    }
                }
            }
        }

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for AthenicDemodulator {
    const VST3_CLASS_ID: [u8; 16] = *b"CharAddDemod\0\0\0\0";

    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Synth];
}

nih_export_vst3!(AthenicDemodulator);
