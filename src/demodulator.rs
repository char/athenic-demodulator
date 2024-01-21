pub const DEMOD_BLOCK_SIZE: usize = 1000;

use crate::{additive_engine::NUM_HARMONICS, DistributionMode};

pub struct CVDemodulator {
    progress: usize,
    prev_harmonic: usize,
    sample_count: usize,
    working_amp_l: [f32; NUM_HARMONICS],
    working_amp_r: [f32; NUM_HARMONICS],
}

impl Default for CVDemodulator {
    fn default() -> Self {
        Self {
            progress: 0,
            prev_harmonic: 1,
            sample_count: 0,
            working_amp_l: [0.0; NUM_HARMONICS],
            working_amp_r: [0.0; NUM_HARMONICS],
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
        distribution_mode: DistributionMode,
        harmonic_ceiling: usize,
        harmonic_offset: usize,
    ) -> Option<([f32; NUM_HARMONICS], [f32; NUM_HARMONICS])> {
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
        }

        for n in 0..in_l.len() {
            let l = in_l[n];
            let r = in_r[n];

            let next_harmonic: usize = match distribution_mode {
                DistributionMode::Exponential => f32::floor(f32::exp(
                    f32::ln(harmonic_ceiling as f32) * (self.progress as f32)
                        / DEMOD_BLOCK_SIZE as f32,
                )) as usize,
                DistributionMode::Linear => f32::floor(
                    (harmonic_ceiling as f32) * (self.progress as f32) / (DEMOD_BLOCK_SIZE as f32),
                ) as usize,
            };
            let next_harmonic = next_harmonic.min(NUM_HARMONICS - 1);

            for harmonic in self.prev_harmonic.max(harmonic_offset)..=next_harmonic {
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
                let mut l = [0.0; NUM_HARMONICS];
                let mut r = [0.0; NUM_HARMONICS];
                l.copy_from_slice(&self.working_amp_l);
                r.copy_from_slice(&self.working_amp_r);
                amps = Some((l, r));
                self.progress = 0;
                self.working_amp_l.fill(0.0);
                self.working_amp_r.fill(0.0);
            }
        }

        amps
    }
}
