use nih_plug::prelude::*;
use std::sync::Arc;

mod envelope;

use crate::envelope::AREnvelope;

struct AdditiveEngine {
    working_harmonic_amplitudes: [f32; 512],
    working_harmonic_phase_shifts: [f32; 512],
    harmonic_amplitudes: [f32; 512],
    harmonic_phase_shifts: [f32; 512],
    harmonic_phases: [f32; 512],
}

impl Default for AdditiveEngine {
    fn default() -> Self {
        Self {
            working_harmonic_amplitudes: [0.0; 512],
            working_harmonic_phase_shifts: [0.0; 512],
            harmonic_amplitudes: [0.0; 512],
            harmonic_phase_shifts: [0.0; 512],
            harmonic_phases: [0.0; 512],
        }
    }
}

struct AthenicDemodulator {
    params: Arc<AthenicDemodulatorParams>,
    engine: AdditiveEngine,
    sample_rate: f32,
    block_progress: usize,
    notes_on: usize,
    current_midi_note: u8,
    bend_amount: f32,
    envelope: AREnvelope,
    envelope_values: Vec<f32>,
    last_block_size: usize,
}

#[derive(Params)]
struct AthenicDemodulatorParams {
    #[id = "block_size"]
    block_size: IntParam,
    #[id = "floor"]
    floor: FloatParam,
    #[id = "ceiling"]
    ceiling: FloatParam,
    #[id = "attack_ms"]
    attack_ms: FloatParam,
    #[id = "release_ms"]
    release_ms: FloatParam,
}

impl Default for AthenicDemodulator {
    fn default() -> Self {
        let mut envelope_values = Vec::new();
        envelope_values.resize_with(4096, || 0.0);

        Self {
            params: Arc::new(AthenicDemodulatorParams::default()),
            engine: AdditiveEngine::default(),
            sample_rate: 44100.0,
            block_progress: 0,
            notes_on: 0,
            current_midi_note: 0,
            bend_amount: 0.0,
            envelope: AREnvelope::default(),
            envelope_values,
            last_block_size: 0,
        }
    }
}

impl Default for AthenicDemodulatorParams {
    fn default() -> Self {
        Self {
            block_size: IntParam::new(
                "block size",
                420, // 420 samples = 105 Hz at 44.1k
                IntRange::Linear { min: 1, max: 512 },
            ),

            floor: FloatParam::new(
                "floor",
                -1.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            ceiling: FloatParam::new(
                "ceiling",
                1.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),

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

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
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
        context.set_latency_samples(self.params.block_size.value() as u32);

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

        let block_size = self.params.block_size.value() as usize;
        if block_size != self.last_block_size {
            context.set_latency_samples(block_size as u32);
        }
        self.last_block_size = block_size;

        self.envelope
            .set_attack_time(self.sample_rate, self.params.attack_ms.value());
        self.envelope
            .set_release_time(self.sample_rate, self.params.release_ms.value());
        self.envelope
            .next_block(&mut self.envelope_values, num_samples);

        for sample_idx in 0..num_samples {
            'events: loop {
                match note_event {
                    Some(event) if (event.timing() as usize) == sample_idx => {
                        // if the event is for the current sample, we should process it:
                        match event {
                            NoteEvent::NoteOn { note, .. } => {
                                if self.notes_on == 0 {
                                    self.block_progress = 0;
                                    for (i, phi) in
                                        self.engine.harmonic_phases.iter_mut().enumerate()
                                    {
                                        *phi = if i % 2 == 0 { 0.75 } else { 0.25 }
                                    }
                                    self.envelope.reset();
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

            if self.block_progress < block_size {
                let mut amplitude = buf[0][sample_idx];
                if amplitude > self.params.ceiling.value() {
                    amplitude = 0.0;
                }
                if amplitude < self.params.floor.value() {
                    amplitude = 0.0;
                }

                self.engine.working_harmonic_amplitudes[self.block_progress] = amplitude;
                self.engine.working_harmonic_phase_shifts[self.block_progress] = buf[1][sample_idx];
            }

            self.block_progress += 1;

            buf[0][sample_idx] = 0.0;
            buf[1][sample_idx] = 0.0;

            if self.block_progress >= block_size {
                self.block_progress = 0;
                self.engine
                    .harmonic_amplitudes
                    .copy_from_slice(&self.engine.working_harmonic_amplitudes);
                self.engine
                    .harmonic_phase_shifts
                    .copy_from_slice(&self.engine.working_harmonic_phase_shifts);
            }

            if self.notes_on > 0 || self.envelope.is_releasing() {
                let fundamental = util::f32_midi_note_to_freq(
                    self.current_midi_note as f32
                        + (self.bend_amount.clamp(0.0, 1.0) * 2.0 - 1.0) * 12.0, // 12.0 = BEND_EXTENTS
                );

                for harmonic_idx in 0..block_size {
                    let frequency = fundamental * (1.0 + harmonic_idx as f32);
                    let step = frequency / self.sample_rate;

                    let gain = self.engine.harmonic_amplitudes[harmonic_idx]
                        * self.envelope_values[sample_idx];
                    let phase = self.engine.harmonic_phases[harmonic_idx];
                    let phase_shift = self.engine.harmonic_phase_shifts[harmonic_idx];

                    self.engine.harmonic_phases[harmonic_idx] = phase + step;

                    if frequency < self.sample_rate / 2.0 {
                        let sample_value =
                            f32::sin((phase + phase_shift) * std::f32::consts::TAU) * gain;

                        buf[0][sample_idx] += sample_value;
                        buf[1][sample_idx] += sample_value;
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
