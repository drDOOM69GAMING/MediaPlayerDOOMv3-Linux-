#![allow(unused_imports)]
#![allow(dead_code)]
// EQ DSP (biquad), presets and rodio source.
use rodio::Source;
pub const EQ_BANDS: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

/// 1-octave graphic-EQ bandwidth. Adjacent ISO bands are one octave apart, so
/// this gives the familiar smooth, overlapping house curve instead of the
/// narrow, disjoint peaks the old custom spacing produced.
pub const EQ_Q: f64 = 1.4;
pub struct EqPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub bands: [f32; 10],
}

/// Curves modelled on the targets used by mix/master and broadcast teams:
/// controlled sub-bass, a clear 1-5 kHz presence region, and no brittle top.
/// Values are dB at 31, 62, 125, 250, 500, 1k, 2k, 4k, 8k and 16k Hz.
pub const EQ_PRESETS: [EqPreset; 20] = [
    EqPreset { name: "Flat", description: "Reference: no coloration", bands: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { name: "Studio", description: "Gentle low-end weight and an open top", bands: [1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 0.5] },
    EqPreset { name: "Harman", description: "Headphone target: sub-bass shelf and presence lift", bands: [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 1.5] },
    EqPreset { name: "Loudness", description: "Smiley curve for low-volume listening", bands: [6.0, 5.0, 2.0, 0.0, -2.0, -1.0, 0.0, 2.0, 4.0, 4.0] },
    EqPreset { name: "Broadcast", description: "Speech intelligibility: cut rumble, lift 2-5 kHz", bands: [-6.0, -4.0, -2.0, 1.0, 3.0, 4.0, 3.0, 1.0, -2.0, -3.0] },
    EqPreset { name: "Podcast", description: "Voice: remove mud, tighten presence, tame sibilance", bands: [-8.0, -5.0, -3.0, 1.0, 3.0, 4.0, 3.0, 1.0, -1.0, -3.0] },
    EqPreset { name: "Vocal Air", description: "Lead vocal clarity and open air", bands: [-5.0, -3.0, -1.0, 0.0, 1.0, 3.0, 4.0, 4.0, 3.0, 2.0] },
    EqPreset { name: "Acoustic", description: "Natural body and string detail", bands: [2.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 1.0] },
    EqPreset { name: "Jazz", description: "Warm body with a smooth top", bands: [3.0, 2.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 1.0] },
    EqPreset { name: "Classical", description: "Wide natural balance without harshness", bands: [1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0] },
    EqPreset { name: "Rock", description: "Punchy lows, controlled mids, open top", bands: [4.0, 3.0, 2.0, 0.0, -1.0, 0.0, 2.0, 3.0, 3.0, 2.0] },
    EqPreset { name: "Metal", description: "Scooped mids, tight bass, aggressive presence", bands: [5.0, 4.0, 2.0, -1.0, -2.0, 0.0, 2.0, 4.0, 4.0, 3.0] },
    EqPreset { name: "Punk", description: "Mid-forward energy and bite", bands: [3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 3.0, 3.0, 2.0, 1.0] },
    EqPreset { name: "Hip-Hop", description: "Deep sub, clear mids, crisp hats", bands: [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 2.0] },
    EqPreset { name: "EDM", description: "Club low-end and synth presence", bands: [6.0, 5.0, 3.0, 1.0, 0.0, 1.0, 2.0, 3.0, 3.0, 2.0] },
    EqPreset { name: "Small Speakers", description: "Night/laptop: cut sub, lift the mids", bands: [-10.0, -7.0, -4.0, 0.0, 2.0, 3.0, 2.0, 0.0, -1.0, -2.0] },
    EqPreset { name: "Bass Boost", description: "Sub and kick emphasis", bands: [8.0, 7.0, 5.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] },
    EqPreset { name: "Treble Boost", description: "Air and detail without excessive sibilance", bands: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 6.0] },
    EqPreset { name: "De-Mud", description: "Cut 250-500 Hz boxiness", bands: [0.0, -1.0, -3.0, -4.0, -2.0, 0.0, 1.0, 1.0, 0.0, 0.0] },
    EqPreset { name: "Dialogue", description: "TV/film dialogue: cut rumble and boom, lift 2-4 kHz", bands: [-8.0, -6.0, -3.0, 0.0, 2.0, 4.0, 4.0, 2.0, -1.0, -3.0] },
];

#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f64, b1: f64, b2: f64, a1: f64, a2: f64,
    x1: f64, x2: f64, y1: f64, y2: f64,
}

impl Biquad {
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1; self.x1 = x;
        self.y2 = self.y1; self.y1 = y;
        y
    }
}

pub fn eq_peaking_coeffs(f0: f64, q: f64, fs: f64, db: f64) -> Biquad {
    // RBJ cookbook peaking EQ, normalized to a0
    let a = 10.0f64.powf(db / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
    let c = w0.cos();
    let s = w0.sin();
    let alpha = s / (2.0 * q);
    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * c;
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * c;
    let a2 = 1.0 - alpha / a;
    Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
}

pub struct EqShared {
    pub gains: [f32; 10],
    pub dirty: bool,
}

pub struct EqSource<S> {
    inner: S,
    ch: u16,
    ch_pos: u16,
    shared: std::sync::Arc<std::sync::Mutex<EqShared>>,
    gains: [f32; 10],
    pub(crate) preamp: f32,
    biquads: [[Biquad; 10]; 2],
}

impl<S: rodio::Source> EqSource<S>
where
    S::Item: rodio::Sample,
    f32: cpal::FromSample<S::Item>,
{
    pub fn new(inner: S, shared: std::sync::Arc<std::sync::Mutex<EqShared>>) -> Self {
        let ch = inner.channels().max(1);
        let fs = inner.sample_rate() as f64;
        let mut gains = [0.0f32; 10];
        if let Ok(st) = shared.lock() {
            gains = st.gains;
        }
        let mut biquads = [[eq_peaking_coeffs(EQ_BANDS[0], EQ_Q, fs, 0.0); 10]; 2];
        for i in 0..10 {
            biquads[0][i] = eq_peaking_coeffs(EQ_BANDS[i], EQ_Q, fs, gains[i] as f64);
        }
        biquads[1] = biquads[0].clone();
        let preamp = eq_auto_preamp(&gains, fs);
        EqSource { inner, ch, ch_pos: 0, shared, gains, preamp, biquads }
    }

    fn sync_gains(&mut self) {
        if let Ok(mut st) = self.shared.lock() {
            if st.dirty {
                st.dirty = false;
                if self.gains != st.gains {
                    self.gains = st.gains;
                    let fs = self.inner.sample_rate() as f64;
                    for i in 0..10 {
                        let b = eq_peaking_coeffs(EQ_BANDS[i], EQ_Q, fs, self.gains[i] as f64);
                        self.biquads[0][i] = b;
                        self.biquads[1][i] = b;
                    }
                    self.preamp = eq_auto_preamp(&self.gains, fs);
                }
            }
        }
    }
}

impl<S: rodio::Source> Iterator for EqSource<S>
where
    S::Item: rodio::Sample,
    f32: cpal::FromSample<S::Item>,
{
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        if self.ch_pos == 0 {
            self.sync_gains();
        }
        let v = self.inner.next()?;
        let mut x = <f32 as cpal::Sample>::from_sample(v) as f64;
        let ci = self.ch_pos as usize;
        for i in 0..10 {
            x = self.biquads[ci][i].process(x);
        }
        // Auto-preamp keeps the curve below 0 dB, but inter-sample peaks can
        // still overshoot. A soft knee avoids the hard-clip edge that made
        // heavily boosted presets sound gritty.
        let out = soft_limit(x * self.preamp as f64) as f32;
        self.ch_pos = (self.ch_pos + 1) % self.ch;
        Some(out)
    }
}

/// Soft-knee output limiter. Transparent below -0.45 dBFS, then gently bends
/// toward full scale instead of clipping square.
pub fn soft_limit(x: f64) -> f64 {
    const KNEE: f64 = 0.95;
    if x > KNEE {
        KNEE + (x - KNEE).tanh() * (1.0 - KNEE)
    } else if x < -KNEE {
        -KNEE - (-x - KNEE).tanh() * (1.0 - KNEE)
    } else {
        x
    }
}

impl<S: rodio::Source> rodio::Source for EqSource<S>
where
    S::Item: rodio::Sample,
    f32: cpal::FromSample<S::Item>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

pub fn eq_response_linear(freq: f64, gains: &[f32; 10], fs: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / fs;
    let c1 = w.cos();
    let s1 = w.sin();
    let c2 = (2.0 * w).cos();
    let s2 = (2.0 * w).sin();
    let nyq = fs / 2.0;
    let mut mag = 1.0;
    for i in 0..10 {
        let db = gains[i] as f64;
        if db.abs() < 0.5 {
            continue;
        }
        let b = eq_peaking_coeffs(EQ_BANDS[i].min(nyq - 100.0).max(20.0), EQ_Q, fs, db);
        let num_re = b.b0 + b.b1 * c1 + b.b2 * c2;
        let num_im = -b.b1 * s1 - b.b2 * s2;
        let den_re = 1.0 + b.a1 * c1 + b.a2 * c2;
        let den_im = -b.a1 * s1 - b.a2 * s2;
        let m = (num_re * num_re + num_im * num_im).sqrt()
            / (den_re * den_re + den_im * den_im).max(1e-12).sqrt();
        mag *= m;
    }
    mag
}

pub fn eq_auto_preamp(gains: &[f32; 10], fs: f64) -> f32 {
    let mut peak = 1.0f64;
    let mut f = 22.0;
    while f <= 20000.0 {
        peak = peak.max(eq_response_linear(f, gains, fs));
        f *= 1.06;
    }
    if peak > 1.0 {
        // 2% headroom on boosted curves; a flat curve stays bit-transparent.
        ((1.0 / peak) * 0.98).min(1.0) as f32
    } else {
        1.0
    }
}

pub fn eq_curve_db(gains: &[f32; 10], fs: f64, points: &mut Vec<(f32, f32)>) {
    const MIN: f64 = 20.0;
    const MAX: f64 = 20000.0;
    let n = points.len();
    for i in 0..n {
        let f = MIN * (MAX / MIN).powf(i as f64 / (n - 1).max(1) as f64);
        let r = eq_response_linear(f, gains, fs).max(1e-9);
        points[i] = (f as f32, (20.0 * r.log10()) as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_names_and_real_descriptions() {
        let mut names = std::collections::HashSet::new();
        for p in EQ_PRESETS.iter() {
            assert!(names.insert(p.name), "duplicate preset {}", p.name);
            assert!(!p.description.trim().is_empty(), "{} has no description", p.name);
            assert!(p.bands.iter().all(|g| g.abs() <= 12.0), "{} exceeds ±12 dB", p.name);
        }
        assert_eq!(EQ_PRESETS[0].name, "Flat");
        assert!(EQ_PRESETS[0].bands.iter().all(|g| *g == 0.0));
    }

    #[test]
    fn bands_are_standard_one_octave_spacing() {
        for w in EQ_BANDS.windows(2) {
            assert!((w[1] / w[0] - 2.0).abs() < 0.03, "{} -> {} is not one octave", w[0], w[1]);
        }
        assert_eq!(EQ_BANDS[0], 31.0);
        assert_eq!(EQ_BANDS[9], 16000.0);
    }

    #[test]
    fn flat_curve_has_no_preamp_loss() {
        assert!((eq_auto_preamp(&[0.0; 10], 44100.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn soft_limiter_is_transparent_then_bends() {
        assert!((soft_limit(0.25) - 0.25).abs() < 1e-12);
        assert!((soft_limit(0.94) - 0.94).abs() < 1e-12);
        assert!(soft_limit(0.97) > 0.95 && soft_limit(0.97) < 0.97);
        assert!(soft_limit(4.0) > 0.99 && soft_limit(4.0) <= 1.0);
        assert!(soft_limit(-4.0) < -0.99 && soft_limit(-4.0) >= -1.0);
    }
}

