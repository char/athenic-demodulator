use demodulator::{CVDemodulator, DEMOD_BLOCK_SIZE};
use nih_plug::prelude::*;
use std::{env, sync::Arc};
use voice::AdditiveVoice;

mod additive_engine;
mod demodulator;
mod envelope;
mod voice;

struct SynthPlugin {
    params: Arc<AthenicDemodulatorParams>,
    voice: AdditiveVoice,
    demodulator: CVDemodulator,
    sample_rate: f32,
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

impl Default for SynthPlugin {
    fn default() -> Self {
        let mut envelope_values = Vec::new();
        envelope_values.resize_with(4096, || 0.0);

        Self {
            params: Arc::new(AthenicDemodulatorParams::default()),
            voice: AdditiveVoice::default(),
            demodulator: CVDemodulator::default(),
            sample_rate: 44100.0,
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

impl Plugin for SynthPlugin {
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
        context.set_latency_samples(DEMOD_BLOCK_SIZE as u32);

        true
    }

    fn reset(&mut self) {
        self.voice.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();

        let buf = buffer.as_slice();
        let (buf_l, buf_r) = buf.split_at_mut(1);
        let buf_l = &mut buf_l[0];
        let buf_r = &mut buf_r[0];

        self.voice
            .envelope
            .set_attack_time(self.sample_rate, self.params.attack_ms.value());
        self.voice
            .envelope
            .set_release_time(self.sample_rate, self.params.release_ms.value());

        let num_partials = self.params.partial_count.value() as usize;
        let partial_offset = self.params.partial_offset.value() as usize;
        let distribution_mode = self.params.distribution_mode.value();

        let cv_floor = self.params.floor.value();
        let cv_ceil = self.params.ceiling.value();
        let cv_bias = self.params.bias.value();

        let mut note_event = context.next_event();
        let mut block_start = 0;
        let mut block_end = (block_start + 64).min(num_samples);

        while block_start < num_samples {
            'events: loop {
                match note_event {
                    Some(event) if (event.timing() as usize) <= block_start => {
                        match event {
                            NoteEvent::NoteOn { note, .. } => {
                                self.voice.note_on(note);
                                self.demodulator.reset();
                            }
                            NoteEvent::NoteOff { .. } => {
                                self.voice.note_off();
                            }
                            NoteEvent::MidiPitchBend { value, .. } => {
                                self.voice.midi_pitch_bend(value);
                            }
                            _ => {}
                        }

                        note_event = context.next_event();
                    }
                    Some(event) if (event.timing() as usize) < block_end => {
                        block_end = event.timing() as usize;
                        break 'events;
                    }
                    _ => break 'events,
                }
            }

            let amps = self.demodulator.submit_samples(
                &buf_l[block_start..block_end],
                &buf_r[block_start..block_end],
                &distribution_mode,
                num_partials,
                partial_offset,
                cv_floor,
                cv_ceil,
                cv_bias,
            );
            if let Some((amp_l, amp_r)) = amps {
                self.voice.engine.submit_amplitudes(&amp_l, &amp_r);
            }

            buf_l[block_start..block_end].fill(0.0);
            buf_r[block_start..block_end].fill(0.0);

            self.voice.process(
                self.sample_rate,
                &mut buf_l[block_start..block_end],
                &mut buf_r[block_start..block_end],
            );

            block_start = block_end;
            block_end = (block_start + 64).min(num_samples);
        }

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for SynthPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"CharAddDemod\0\0\0\0";

    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Synth];
}

nih_export_vst3!(SynthPlugin);
