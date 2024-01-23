pub const DEMOD_BLOCK_SIZE: usize = 1050; // 42Hz @ 44.1KHz s.r.

use crate::{additive_engine::MAX_HARMONICS, DistributionMode};

pub struct CVDemodulator {
    progress: usize,
    prev_harmonic: usize,
    sample_count: usize,
    working_amp_l: [f32; MAX_HARMONICS],
    working_amp_r: [f32; MAX_HARMONICS],
}

impl Default for CVDemodulator {
    fn default() -> Self {
        Self {
            progress: 0,
            prev_harmonic: 1,
            sample_count: 0,
            working_amp_l: [0.0; MAX_HARMONICS],
            working_amp_r: [0.0; MAX_HARMONICS],
        }
    }
}

impl CVDemodulator {
    pub fn reset(&mut self) {
        self.progress = 0;
        self.prev_harmonic = 1;
        self.sample_count = 0;
    }

    pub fn submit_samples(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        distribution_mode: &DistributionMode,
        harmonic_count: usize,
        harmonic_offset: usize,
        floor: f32,
        ceiling: f32,
        bias: f32,
    ) -> Option<([f32; MAX_HARMONICS], [f32; MAX_HARMONICS])> {
        assert_eq!(
            in_l.len(),
            in_r.len(),
            "channel buffers should have matching sample counts"
        );

        let mut amps = None;

        if self.progress >= DEMOD_BLOCK_SIZE {
            self.progress = 0;
            self.working_amp_l.fill(0.0);
            self.working_amp_r.fill(0.0);
            self.prev_harmonic = 1;
        }

        for n in 0..in_l.len() {
            let mut l = in_l[n] + bias;
            if l > ceiling {
                l = 0.0;
            }
            if l < floor {
                l = 0.0;
            }
            let mut r = in_r[n] + bias;
            if r > ceiling {
                r = 0.0;
            }
            if r < floor {
                r = 0.0;
            }

            let next_harmonic: usize = match distribution_mode {
                DistributionMode::Exponential => f32::floor(f32::exp(
                    f32::ln(harmonic_count as f32) * (self.progress as f32)
                        / DEMOD_BLOCK_SIZE as f32,
                )) as usize,
                DistributionMode::Linear => f32::floor(
                    (harmonic_count as f32) * (self.progress as f32) / (DEMOD_BLOCK_SIZE as f32),
                ) as usize,
            } + harmonic_offset;

            for harmonic in
                self.prev_harmonic.max(harmonic_offset).max(1)..=next_harmonic.min(MAX_HARMONICS)
            {
                let l = l * l * l.signum();
                let r = r * r * r.signum();

                if harmonic != self.prev_harmonic {
                    self.sample_count = 0;
                    self.working_amp_l[harmonic - 1] = l;
                    self.working_amp_r[harmonic - 1] = r;
                } else {
                    // moving average
                    self.sample_count += 1;
                    self.working_amp_l[harmonic - 1] =
                        (self.working_amp_l[harmonic - 1] * (self.sample_count as f32) + l)
                            / (self.sample_count as f32);
                    self.working_amp_r[harmonic - 1] =
                        (self.working_amp_r[harmonic - 1] * (self.sample_count as f32) + r)
                            / (self.sample_count as f32);
                }

                self.prev_harmonic = harmonic;
            }

            self.progress += 1;
            if self.progress >= DEMOD_BLOCK_SIZE {
                let mut l = [0.0; MAX_HARMONICS];
                let mut r = [0.0; MAX_HARMONICS];
                l.copy_from_slice(&self.working_amp_l);
                r.copy_from_slice(&self.working_amp_r);
                amps = Some((l, r));
                self.progress = 0;
                self.working_amp_l.fill(0.0);
                self.working_amp_r.fill(0.0);
                self.prev_harmonic = 1;
            }
        }

        amps
    }
}
