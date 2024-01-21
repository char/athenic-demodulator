pub const NUM_HARMONICS: usize = 512;

pub struct AdditiveEngine {
    pub phases: [f64; NUM_HARMONICS],
    pub amp_l: [f32; NUM_HARMONICS],
    pub amp_r: [f32; NUM_HARMONICS],
    last_amp_l: [f32; NUM_HARMONICS],
    last_amp_r: [f32; NUM_HARMONICS],
}

impl Default for AdditiveEngine {
    fn default() -> Self {
        Self {
            phases: [0.0; NUM_HARMONICS],
            amp_l: [0.0; NUM_HARMONICS],
            amp_r: [0.0; NUM_HARMONICS],
            last_amp_l: [0.0; NUM_HARMONICS],
            last_amp_r: [0.0; NUM_HARMONICS],
        }
    }
}

impl AdditiveEngine {
    pub fn submit_amplitudes(&mut self, amp_l: &[f32], amp_r: &[f32]) {
        self.amp_l.copy_from_slice(amp_l);
        self.amp_r.copy_from_slice(amp_r);
    }

    pub fn reset_slew_tracking(&mut self) {
        self.last_amp_l.fill(0.0);
        self.last_amp_r.fill(0.0);
    }

    #[allow(clippy::needless_range_loop)] // autovectorization
    pub fn generate_samples(
        &mut self,
        i_freqs: &[f64; NUM_HARMONICS],
        sample_rate: f32,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) {
        assert_eq!(
            out_l.len(),
            out_r.len(),
            "channel output buffers must match length"
        );

        let sr_f64 = sample_rate as f64;

        for n in 0..out_l.len() {
            let mut samp_l: f32 = 0.0;
            let mut samp_r: f32 = 0.0;

            for i in 0..NUM_HARMONICS {
                let freq = i_freqs[i];
                let step = freq / sr_f64;
                let phase = &mut self.phases[i];

                *phase += step;
                if *phase > 2.0 {
                    *phase -= 2.0;
                }

                if freq >= 20.0 && freq <= sr_f64 / 2.0 {
                    let v = f64::sin(*phase * std::f64::consts::TAU);

                    let slew_threshold = 12.5 / sample_rate;
                    let amp_l = self.amp_l[n];
                    let amp_r = self.amp_r[n];
                    let last_amp_l = self.last_amp_l[i];
                    let last_amp_r = self.last_amp_r[i];
                    let delta_amp_l = amp_l - last_amp_l;
                    let delta_amp_r = amp_r - last_amp_r;
                    let amp_l = last_amp_l + delta_amp_l.clamp(-slew_threshold, slew_threshold);
                    let amp_r = last_amp_r + delta_amp_r.clamp(-slew_threshold, slew_threshold);
                    self.last_amp_l[n] = amp_l;
                    self.last_amp_r[n] = amp_r;

                    let basic_gain = 1.0 / (i as f32 + 1.0);
                    let basic_gain = basic_gain.sqrt() * 2.0;

                    samp_l += v as f32 * amp_l * basic_gain;
                    samp_r += v as f32 * amp_r * basic_gain;
                }
            }

            out_l[n] += samp_l;
            out_r[n] += samp_r;
        }
    }
}
