const PI: f32 = std::f32::consts::PI;

/// The GBA's native APU sample rate.
const GBA_NATIVE_RATE: f32 = 32_768.0;

fn cosine_interpolation(y1: f32, y2: f32, phase: f32) -> f32 {
    let mu2 = (1.0 - (PI * phase).cos()) / 2.0;
    y1 * (1.0 - mu2) + y2 * mu2
}

/// Resamples stereo audio from the GBA's native 32768 Hz to the audio device output
/// rate using cosine interpolation.
pub struct CosineResampler {
    last_left: f32,
    last_right: f32,
    phase: f32,
    pub out_freq: f32,
}

impl CosineResampler {
    pub fn new(out_freq: f32) -> Self {
        Self {
            last_left: 0.0,
            last_right: 0.0,
            phase: 0.0,
            out_freq,
        }
    }

    /// Feed one stereo native-rate (32768 Hz) sample pair; appends interleaved
    /// [L, R, L, R, ...] resampled output into `buf`.
    pub fn feed(&mut self, left: f32, right: f32, buf: &mut Vec<f32>) {
        while self.phase < 1.0 {
            buf.push(cosine_interpolation(self.last_left, left, self.phase));
            buf.push(cosine_interpolation(self.last_right, right, self.phase));
            self.phase += GBA_NATIVE_RATE / self.out_freq;
        }
        self.phase -= 1.0;
        self.last_left = left;
        self.last_right = right;
    }
}
