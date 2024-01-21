use crate::{
    additive_engine::{AdditiveEngine, NUM_HARMONICS},
    envelope::AREnvelope,
};

const BEND_RANGE: f64 = 12.0;
const VOICE_BLOCK_SIZE: usize = 32;

pub struct AdditiveVoice {
    pub engine: AdditiveEngine,
    pub envelope: AREnvelope,
    current_midi_note: u8,
    bend_value: f32,
    note_on: bool,
}

impl Default for AdditiveVoice {
    fn default() -> Self {
        let mut this = Self {
            engine: Default::default(),
            envelope: Default::default(),
            current_midi_note: 0,
            bend_value: 0.5,
            note_on: false,
        };
        this.reset_phases();
        this
    }
}

impl AdditiveVoice {
    fn reset_phases(&mut self) {
        for (i, phi) in self.engine.phases.iter_mut().enumerate() {
            *phi = NUM_HARMONICS as f64 / (i + 1) as f64;
        }
    }

    pub fn note_on(&mut self, note: u8) {
        self.envelope.reset();
        self.current_midi_note = note;
        self.note_on = true;
    }

    pub fn note_off(&mut self) {
        self.envelope.start_release();
        self.note_on = false;
    }

    pub fn midi_pitch_bend(&mut self, value: f32) {
        self.bend_value = value;
    }

    pub fn process(&mut self, sample_rate: f32, out_l: &mut [f32], out_r: &mut [f32]) {
        if !(self.note_on || self.envelope.is_releasing()) {
            return;
        }

        assert_eq!(
            out_l.len(),
            out_r.len(),
            "channel output buffers must match length"
        );
        assert!(
            out_l.len() % 32 == 0,
            "channel output buffer size must be multiple of 32"
        );

        let note = self.current_midi_note as f64
            + (self.bend_value.clamp(0.0, 1.0) * 2.0 - 1.0) as f64 * BEND_RANGE;
        let fundamental = 2.0f64.powf((note - 69.0) / 12.0) * 440.0;
        let mut i_freqs = [0.0; NUM_HARMONICS];
        for n in 0..NUM_HARMONICS {
            i_freqs[n] = fundamental * (n + 1) as f64;
        }

        let mut i = 0;
        while i < out_l.len() {
            let mut envelope_values = [0.0; 32];
            self.envelope.next_block(&mut envelope_values, 32);

            let mut buf_l = [0.0; 32];
            let mut buf_r = [0.0; 32];
            self.engine
                .generate_samples(&i_freqs, sample_rate, &mut buf_l, &mut buf_r);

            for smp in 0..VOICE_BLOCK_SIZE {
                out_l[i + smp] += buf_l[smp] * envelope_values[smp];
                out_r[i + smp] += buf_r[smp] * envelope_values[smp];
            }

            i += 32;
        }

        if !self.note_on && !self.envelope.is_releasing() {
            self.reset_phases();
            self.engine.reset_slew_tracking();
        }
    }
}
