use nih_plug::nih_debug_assert;

#[derive(Debug, Default)]
pub struct AREnvelope {
    state: f32,

    attack_coeff: f32,
    release_coeff: f32,

    releasing: bool,
}

impl AREnvelope {
    pub fn set_attack_time(&mut self, sample_rate: f32, time_ms: f32) {
        self.attack_coeff = (-1.0 / (time_ms / 1000.0 * sample_rate)).exp();
    }

    pub fn set_release_time(&mut self, sample_rate: f32, time_ms: f32) {
        self.release_coeff = (-1.0 / (time_ms / 1000.0 * sample_rate)).exp();
    }

    pub fn reset(&mut self) {
        self.state = 0.0;
        self.releasing = false;
    }

    pub fn current(&self) -> f32 {
        self.state
    }

    pub fn next_block(&mut self, block_values: &mut [f32], block_len: usize) {
        nih_debug_assert!(block_values.len() >= block_len);
        for value in block_values.iter_mut().take(block_len) {
            let (target, t) = if self.releasing {
                (0.0, self.release_coeff)
            } else {
                (1.0, self.attack_coeff)
            };

            let new = (self.state * t) + (target * (1.0 - t));
            self.state = new;

            *value = new;
        }
    }

    pub fn start_release(&mut self) {
        self.releasing = true;
    }

    pub fn is_releasing(&self) -> bool {
        self.releasing && self.state >= 0.001
    }
}
