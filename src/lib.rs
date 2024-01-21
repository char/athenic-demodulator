use nih_plug::prelude::*;
use std::{env, sync::Arc};

mod additive_engine;
mod demodulator;
mod envelope;
mod voice;

use crate::envelope::AREnvelope;

const BLOCK_SIZE: usize = 1050; // 1050 samples = 42Hz wave period at 44.1k

struct AdditiveEngine {
    prev_working_harmonic: usize,
    working_harmonic_amplitudes_l: [f32; 512],
    working_harmonic_amplitudes_r: [f32; 512],
    block_harmonic_amplitudes_l: [f32; 512],
    block_harmonic_amplitudes_r: [f32; 512],
    prev_block_harmonic_amplitudes_l: [f32; 512],
    prev_block_harmonic_amplitudes_r: [f32; 512],
    last_sample_harmonic_amplitude_l: [f32; 512],
    last_sample_harmonic_amplitude_r: [f32; 512],
    harmonic_phases: [f32; 512],
    working_progress: usize,
    playback_progress: usize,
    harmonic_sample_count: usize,
    was_emitting: bool,
}

impl AdditiveEngine {
    pub fn reset_phases(&mut self) {
        // TODO: phase modes ?
        for (i, phi) in self.harmonic_phases.iter_mut().enumerate() {
            *phi = 512.0 / (i + 1) as f32;
        }
    }
}

impl Default for AdditiveEngine {
    fn default() -> Self {
        Self {
            prev_working_harmonic: 1,
            working_harmonic_amplitudes_l: [0.0; 512],
            working_harmonic_amplitudes_r: [0.0; 512],
            block_harmonic_amplitudes_l: [0.0; 512],
            block_harmonic_amplitudes_r: [0.0; 512],
            prev_block_harmonic_amplitudes_l: [0.0; 512],
            prev_block_harmonic_amplitudes_r: [0.0; 512],
            last_sample_harmonic_amplitude_l: [0.0; 512],
            last_sample_harmonic_amplitude_r: [0.0; 512],
            harmonic_phases: [0.0; 512],
            working_progress: 0,
            playback_progress: 0,
            harmonic_sample_count: 0,
            was_emitting: false,
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

#[derive(Enum, PartialEq, Debug)]
enum DistributionMode {
    Exponential,
    Linear,
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
    #[id = "distribution_mode"]
    distribution_mode: EnumParam<DistributionMode>,
}

impl Default for AthenicDemodulator {
    fn default() -> Self {
        let mut envelope_values = Vec::new();
        envelope_values.resize_with(4096, || 0.0);

        let mut engine = AdditiveEngine::default();
        engine.reset_phases();

        Self {
            params: Arc::new(AthenicDemodulatorParams::default()),
            engine,
            sample_rate: 44100.0,
            notes_on: 0,
            current_midi_note: 0,
            bend_amount: 0.5,
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
            distribution_mode: EnumParam::new("distribution mode", DistributionMode::Exponential),
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
        let distribution_mode = self.params.distribution_mode.value();

        for sample_idx in 0..num_samples {
            'events: loop {
                match note_event {
                    Some(event) if (event.timing() as usize) < sample_idx => {
                        // if the event already passed, try the next one:
                        note_event = context.next_event();
                    }
                    Some(event) if (event.timing() as usize) == sample_idx => {
                        // if the event is for the current sample, we should process it:
                        match event {
                            NoteEvent::NoteOn { note, .. } => {
                                // TODO: polyphony???

                                self.envelope.reset();
                                self.engine.working_progress = 0;
                                self.engine.prev_working_harmonic = 1;

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
                    _ => {
                        // if the event is after the current sample, just hold on to it:
                        break 'events;
                    }
                }
            }

            if self.engine.working_progress < BLOCK_SIZE {
                // TODO: improve distribution
                let next_harmonic: usize = match distribution_mode {
                    DistributionMode::Exponential => f32::floor(f32::exp(
                        f32::ln(num_partials as f32) * (self.engine.working_progress as f32)
                            / BLOCK_SIZE as f32,
                    )) as usize,

                    DistributionMode::Linear => f32::floor(
                        (self.engine.working_progress as f32) * (num_partials as f32)
                            / (BLOCK_SIZE as f32),
                    ) as usize,
                } + partial_offset;

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

                for harmonic in
                    self.engine.prev_working_harmonic.max(partial_offset)..=next_harmonic
                {
                    if harmonic >= 512 {
                        break;
                    }

                    let l = amplitude_l * amplitude_l * amplitude_l.signum();
                    let r = amplitude_r * amplitude_r * amplitude_r.signum();

                    if harmonic != self.engine.prev_working_harmonic {
                        self.engine.working_harmonic_amplitudes_l[harmonic - 1] = l;
                        self.engine.working_harmonic_amplitudes_r[harmonic - 1] = r;
                        self.engine.harmonic_sample_count = 1;
                    } else {
                        // moving average
                        self.engine.harmonic_sample_count += 1;
                        self.engine.working_harmonic_amplitudes_l[harmonic - 1] =
                            (self.engine.working_harmonic_amplitudes_l[harmonic - 1]
                                * (self.engine.harmonic_sample_count as f32 - 1.0)
                                + l)
                                / (self.engine.harmonic_sample_count as f32);
                        self.engine.working_harmonic_amplitudes_r[harmonic - 1] =
                            (self.engine.working_harmonic_amplitudes_r[harmonic - 1]
                                * (self.engine.harmonic_sample_count as f32 - 1.0)
                                + r)
                                / (self.engine.harmonic_sample_count as f32);
                    }

                    self.engine.prev_working_harmonic = harmonic;
                }
            }

            self.engine.working_progress += 1;

            if self.engine.working_progress >= BLOCK_SIZE {
                self.engine.working_progress = 0;

                self.engine
                    .prev_block_harmonic_amplitudes_l
                    .copy_from_slice(&self.engine.block_harmonic_amplitudes_l);
                self.engine
                    .prev_block_harmonic_amplitudes_r
                    .copy_from_slice(&self.engine.block_harmonic_amplitudes_r);

                self.engine
                    .block_harmonic_amplitudes_l
                    .copy_from_slice(&self.engine.working_harmonic_amplitudes_l);
                self.engine
                    .block_harmonic_amplitudes_r
                    .copy_from_slice(&self.engine.working_harmonic_amplitudes_r);

                self.engine.working_harmonic_amplitudes_l.fill(0.0);
                self.engine.working_harmonic_amplitudes_r.fill(0.0);

                self.engine.prev_working_harmonic = 1;
            }

            buf[0][sample_idx] = 0.0;
            buf[1][sample_idx] = 0.0;

            self.engine.playback_progress += 1;
            let playback_t = self.engine.playback_progress as f32 / BLOCK_SIZE as f32;

            if self.notes_on > 0 || self.envelope.is_releasing() {
                self.engine.was_emitting = true;

                let fundamental = util::f32_midi_note_to_freq(
                    self.current_midi_note as f32
                        + (self.bend_amount.clamp(0.0, 1.0) * 2.0 - 1.0) * 12.0, // 12.0 = BEND_EXTENTS
                );

                // 1. calculate amplitude for all partials (with slew clamping for de-clicking)
                // 2. the nyquist frequency can be calculated in advance so that we know how many partials we need
                // 3. all harmonics get sampled & phase-stepped

                let mut amplitudes_l: [f32; 512] = [0.0; 512];
                let mut amplitudes_r: [f32; 512] = [0.0; 512];

                for i in 0..512 {
                    let amp_l = self.engine.prev_block_harmonic_amplitudes_l[i]
                        * (1.0 - playback_t)
                        + self.engine.block_harmonic_amplitudes_l[i] * playback_t;
                    let amp_r = self.engine.prev_block_harmonic_amplitudes_r[i]
                        * (1.0 - playback_t)
                        + self.engine.block_harmonic_amplitudes_r[i] * playback_t;
                    let amp_slew_threshold = 12.5 / self.sample_rate;
                    let last_amp_l = self.engine.last_sample_harmonic_amplitude_l[i];
                    let last_amp_r = self.engine.last_sample_harmonic_amplitude_r[i];
                    let delta_amp_l = amp_l - last_amp_l;
                    let delta_amp_r = amp_r - last_amp_r;
                    let amp_l =
                        last_amp_l + delta_amp_l.clamp(-amp_slew_threshold, amp_slew_threshold);
                    let amp_r =
                        last_amp_r + delta_amp_r.clamp(-amp_slew_threshold, amp_slew_threshold);
                    self.engine.last_sample_harmonic_amplitude_l[i] = amp_l;
                    self.engine.last_sample_harmonic_amplitude_r[i] = amp_r;

                    amplitudes_l[i] = amp_l;
                    amplitudes_r[i] = amp_r;
                }

                let mut l = 0.0;
                let mut r = 0.0;
                let mut frequency = 0.0;

                let nyquist = self.sample_rate / 2.0;
                let nyquist_harmonic_idx = f32::ceil(nyquist / fundamental) as usize;
                let floor_harmonic_idx = f32::ceil(20.0 / fundamental) as usize;

                for (harmonic_idx, phase) in self.engine.harmonic_phases
                    [0.max(floor_harmonic_idx)..num_partials.min(nyquist_harmonic_idx)]
                    .iter_mut()
                    .enumerate()
                {
                    frequency += fundamental;
                    let step = frequency / self.sample_rate;
                    *phase += step;
                    let v = f32::sin(*phase * std::f32::consts::TAU);

                    let saw_gain = 1.0 / (harmonic_idx as f32 + 1.0);
                    let basic_gain = saw_gain.sqrt() * 2.0;
                    let sample_l = v * basic_gain * amplitudes_l[harmonic_idx];
                    let sample_r = v * basic_gain * amplitudes_r[harmonic_idx];

                    l += sample_l;
                    r += sample_r;
                }

                let envelope = self.envelope_values[sample_idx];
                buf[0][sample_idx] = l * envelope * 0.5;
                buf[1][sample_idx] = r * envelope * 0.5;
            } else if self.engine.was_emitting {
                self.engine.was_emitting = false;

                self.engine.reset_phases();
                self.engine.last_sample_harmonic_amplitude_l.fill(0.0);
                self.engine.last_sample_harmonic_amplitude_r.fill(0.0);
                self.engine.block_harmonic_amplitudes_l.fill(0.0);
                self.engine.block_harmonic_amplitudes_r.fill(0.0);
                self.engine.prev_block_harmonic_amplitudes_l.fill(0.0);
                self.engine.prev_block_harmonic_amplitudes_r.fill(0.0);
            }

            if self.engine.playback_progress >= BLOCK_SIZE {
                self.engine.playback_progress = 0;
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
