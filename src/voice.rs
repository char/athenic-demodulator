use crate::{
    additive_engine::{AdditiveEngine, MAX_HARMONICS},
    envelope::AREnvelope,
    BasicGainMode,
};

const BEND_RANGE: f64 = 12.0;
const VOICE_BLOCK_SIZE: usize = 32;

pub struct AdditiveVoice {
    pub engine: AdditiveEngine,
    pub envelope: AREnvelope,
    current_midi_note: u8,
    bend_value: f32,
    notes_on: usize,
}

impl Default for AdditiveVoice {
    fn default() -> Self {
        let mut this = Self {
            engine: Default::default(),
            envelope: Default::default(),
            current_midi_note: 0,
            bend_value: 0.5,
            notes_on: 0,
        };
        this.reset_phases();
        this
    }
}

impl AdditiveVoice {
    fn reset_phases(&mut self) {
        for (i, phi) in self.engine.phases.iter_mut().enumerate() {
            *phi = MAX_HARMONICS as f64 / (i + 1) as f64;
        }
    }

    pub fn note_on(&mut self, note: u8) {
        if self.notes_on == 0 {
            self.envelope.reset();
        }
        self.current_midi_note = note;
        self.notes_on += 1;
    }

    pub fn note_off(&mut self) {
        self.notes_on = self.notes_on.saturating_sub(1);
        if self.notes_on == 0 {
            self.envelope.start_release();
        }
    }

    pub fn midi_pitch_bend(&mut self, value: f32) {
        self.bend_value = value;
    }

    pub fn reset(&mut self) {
        self.envelope.reset();
        self.notes_on = 0;
        self.engine.reset_slew_tracking();
    }

    pub fn process(
        &mut self,
        sample_rate: f32,
        out_l: &mut [f32],
        out_r: &mut [f32],
        basic_gain_mode: &BasicGainMode,
        slew_limiting: bool,
    ) {
        if !(self.notes_on > 0 || self.envelope.is_releasing()) {
            return;
        }

        assert_eq!(
            out_l.len(),
            out_r.len(),
            "channel output buffers must match length"
        );

        let note = self.current_midi_note as f64
            + (self.bend_value.clamp(0.0, 1.0) * 2.0 - 1.0) as f64 * BEND_RANGE;
        let fundamental = 2.0f64.powf((note - 69.0) / 12.0) * 440.0;
        let mut i_freqs = [0.0; MAX_HARMONICS];
        for n in 0..MAX_HARMONICS {
            i_freqs[n] = fundamental * (n + 1) as f64;
        }

        let mut i = 0;
        while i < out_l.len() {
            let mut envelope_values = [0.0; VOICE_BLOCK_SIZE];
            let mut buf_l = [0.0; VOICE_BLOCK_SIZE];
            let mut buf_r = [0.0; VOICE_BLOCK_SIZE];

            let block_len = (out_l.len() - i).min(VOICE_BLOCK_SIZE);
            let envelope_values = &mut envelope_values[0..block_len];
            let buf_l = &mut buf_l[0..block_len];
            let buf_r = &mut buf_r[0..block_len];

            self.envelope.next_block(envelope_values, block_len);
            self.engine.generate_samples(
                &i_freqs,
                sample_rate,
                buf_l,
                buf_r,
                basic_gain_mode,
                slew_limiting,
            );

            for smp in 0..block_len {
                out_l[i + smp] += buf_l[smp] * envelope_values[smp];
                out_r[i + smp] += buf_r[smp] * envelope_values[smp];
            }

            i += block_len;
        }

        if self.notes_on == 0 && !self.envelope.is_releasing() {
            self.reset_phases();
            self.engine.reset_slew_tracking();
        }
    }
}
