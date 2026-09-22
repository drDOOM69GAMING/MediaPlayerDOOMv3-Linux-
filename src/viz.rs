#![allow(unused_imports)]
#![allow(dead_code)]
// Visualizer FFT + rodio tap source.
//
// The tap reads the ACTUAL decoded samples being played (post-EQ) and runs a
// real FFT, so the bars genuinely follow the music. Improvements over the
// original version:
//   * stereo mixing - both channels are averaged per frame (was left-only)
//   * per-band auto gain - each band is normalized against its own typical
//     level, so quiet AND loud masters fill the same range, and the treble
//     bars follow real treble content instead of being pinned down by bass
//   * fast envelope - instant peak-hold attack with a ~0.18 s release keeps
//     the bars tight to the music (snap up on hits, settle between beats)
//   * tempo tracking - onset intervals are measured and median-filtered into
//     a BPM estimate; a beat pulse fires on the detected beat grid so the
//     speaker woofers thump in time with the music.
use rodio::Source;
use crate::consts::{VIZ_BANDS, VIZ_HOP, VIZ_WIN};
#[derive(Clone, Copy)]
pub struct VizState {
    pub bars: [f32; VIZ_BANDS],
    pub rms: f32,
    pub onset: f32,
    pub bpm: f32,
    pub beat_pulse: f32,
}

impl Default for VizState {
    fn default() -> Self {
        VizState {
            bars: [0.0; VIZ_BANDS],
            rms: 0.0,
            onset: 0.0,
            bpm: 120.0,
            beat_pulse: 0.0,
        }
    }
}

pub fn viz_fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    if !n.is_power_of_two() {
        return;
    }
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while (j & bit) != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wlen_r, wlen_i) = (ang.cos(), ang.sin());
        let half = len >> 1;
        let mut i = 0usize;
        while i < n {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..half {
                let er = re[i + k + half];
                let ei = im[i + k + half];
                let tr = er * wr - ei * wi;
                let ti = er * wi + ei * wr;
                re[i + k + half] = re[i + k] - tr;
                im[i + k + half] = im[i + k] - ti;
                re[i + k] += tr;
                im[i + k] += ti;
                let nwr = wr * wlen_r - wi * wlen_i;
                wi = wr * wlen_i + wi * wlen_r;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

pub struct VizTap<S> {
    inner: S,
    out: std::sync::Arc<std::sync::Mutex<VizState>>,
    ch: u16,
    edges: [f32; VIZ_BANDS + 1],
    ring: Vec<f32>,
    pos: usize,
    frame_pos: u16,
    frame_acc: f32,
    since: usize,
    fft_re: Vec<f32>,
    fft_im: Vec<f32>,
    acc_rms: f32,
    acc_cnt: usize,
    prev_rms: f32,
    smooth_rms: f32,
    bar_typ: [f32; VIZ_BANDS],
    rms_peak: f32,
    band_agc: [f32; VIZ_BANDS],
    elapsed: f32,
    last_onset: f32,
    intervals: Vec<f32>,
    bpm: f32,
    beat_phase: f32,
}

impl<S: rodio::Source> VizTap<S>
where
    S::Item: rodio::Sample,
{
    pub fn new(inner: S, out: std::sync::Arc<std::sync::Mutex<VizState>>) -> Self {
        let mut edges = [0.0f32; VIZ_BANDS + 1];
        for i in 0..=VIZ_BANDS {
            edges[i] = 30.0f32 * (16000.0f32 / 30.0f32).powf(i as f32 / VIZ_BANDS as f32);
        }
        let ch = inner.channels().max(1);
        VizTap {
            inner,
            out,
            ch,
            edges,
            ring: vec![0.0; VIZ_WIN],
            pos: 0,
            frame_pos: 0,
            frame_acc: 0.0,
            since: 0,
            fft_re: vec![0.0; VIZ_WIN],
            fft_im: vec![0.0; VIZ_WIN],
            acc_rms: 0.0,
            acc_cnt: 0,
            prev_rms: 0.0,
            smooth_rms: 0.0,
            bar_typ: [0.0001; VIZ_BANDS],
            rms_peak: 0.02,
            band_agc: [-40.0; VIZ_BANDS],
            elapsed: 0.0,
            last_onset: 0.0,
            intervals: Vec::with_capacity(8),
            bpm: 120.0,
            beat_phase: 0.0,
        }
    }

    fn analyze(&mut self) {
        let n = self.ring.len();
        for i in 0..n {
            let idx = (self.pos + i) % n;
            let h = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos();
            self.fft_re[i] = self.ring[idx] * h;
            self.fft_im[i] = 0.0;
        }
        viz_fft(&mut self.fft_re, &mut self.fft_im);
        let sr = self.inner.sample_rate().max(1) as f32;
        let fbin = sr / n as f32;
        let mut bars = [0.0f32; VIZ_BANDS];
        // Analysis rate in hops per second (used for real-time smoothing consts).
        let rate = (sr / VIZ_HOP as f32).max(1.0);
        // Snappy release (~0.22 s) for the bar envelope. Attack is instant
        // peak-hold so a hit snaps the bar up the same hop it lands; the slow
        // "typical" per-band level is tracked by the AGC below.
        let kd = 1.0 - (-1.0 / (0.22 * rate)).exp();
        let kagc = 1.0 - (-1.0 / (2.0 * rate)).exp();
        for b in 0..VIZ_BANDS {
            let k0 = ((self.edges[b] / fbin).ceil() as usize).max(1);
            let k1 = ((self.edges[b + 1] / fbin).floor() as usize).min(n / 2 - 1);
            let mut power = 0.0f64;
            for k in k0..=k1 {
                let re = self.fft_re[k] as f64;
                let im = self.fft_im[k] as f64;
                power += re * re + im * im;
            }
            let db = 10.0 * ((power / (k1 - k0 + 1).max(1) as f64) + 1e-12).log10() as f32;
            // Per-band auto-gain: normalize each band against ITS OWN typical
            // level so treble ranges move with real treble content instead of
            // being pinned to zero by bass/mid dominance. CENTERED mapping so a
            // band at its average level sits ~1/3 up; only real hits +12..16 dB
            // reach the top - bars track the song instead of living maxed out.
            let agc = self.band_agc[b];
            let src = ((db - agc + 10.0) / 26.0).clamp(0.0, 1.0);
            // Absolute gate: frequency ranges that are essentially silent
            // (above the file's lowpass, dither-level noise) must not float at
            // mid-height just because they are normalized - they stay at zero
            // until real signal shows up.
            let gate = ((db + 55.0) / 25.0).clamp(0.0, 1.0);
            let src = src * gate;
            // Peak-hold attack, fast release: bars snap up with the music and
            // settle between beats instead of drifting on a slow average.
            let typ = self.bar_typ[b];
            if src > typ {
                self.bar_typ[b] = src;
            } else {
                self.bar_typ[b] = typ + (src - typ) * kd;
            }
            bars[b] = self.bar_typ[b];
            // Age the per-band follower AFTER the bar used the old value.
            self.band_agc[b] = agc + (db - agc) * kagc;
        }

        let rms_win = ((self.acc_rms / self.acc_cnt.max(1) as f32).sqrt() * 4.0).min(1.0);
        let peak = self.rms_peak.max(rms_win * 0.995);
        self.rms_peak = peak.max(0.02);
        let rms_n = (rms_win / self.rms_peak.max(0.02)).clamp(0.0, 1.0);
        self.smooth_rms = self.smooth_rms * 0.75 + rms_n * 0.25;
        let onset = (self.smooth_rms - self.prev_rms).abs();
        self.prev_rms = self.smooth_rms;

        // Tempo tracking: strong onsets measure intervals; a median of the last
        // few yields a stable BPM even with stray transients.
        self.elapsed += VIZ_HOP as f32 / sr;
        if onset > 0.5 && self.last_onset > 0.0 {
            let iv = self.elapsed - self.last_onset;
            if iv > 0.20 && iv < 2.0 {
                self.intervals.push(iv);
                if self.intervals.len() > 8 {
                    self.intervals.remove(0);
                }
                if !self.intervals.is_empty() {
                    let mut sorted = self.intervals.clone();
                    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    let med = sorted[sorted.len() / 2];
                    let b = (60.0 / med).clamp(60.0, 200.0);
                    // Smooth so the estimate glides instead of jumping.
                    self.bpm = self.bpm * 0.6 + b * 0.4;
                }
            }
        }
        if onset > 0.5 {
            // Pull the beat phase toward the transient so the pulse aligns.
            self.beat_phase *= 0.4;
            self.last_onset = self.elapsed;
        }
        let bps = self.bpm / 60.0;
        self.beat_phase = (self.beat_phase + (VIZ_HOP as f32 / sr) * bps).fract();
        // A decaying pulse that fires on each detected beat.
        let beat_pulse = (-self.beat_phase * 5.0).exp() * rms_n;

        if let Ok(mut st) = self.out.lock() {
            st.bars = bars;
            st.rms = rms_n;
            st.onset = onset.clamp(0.0, 1.0);
            st.bpm = self.bpm;
            st.beat_pulse = beat_pulse.clamp(0.0, 1.0);
        }
        self.acc_rms = 0.0;
        self.acc_cnt = 0;
        self.since = 0;
    }
}

impl<S: rodio::Source> Iterator for VizTap<S>
where
    S::Item: rodio::Sample,
    f32: cpal::FromSample<S::Item>,
{
    type Item = S::Item;
    fn next(&mut self) -> Option<Self::Item> {
        let v = self.inner.next()?;
        self.frame_acc += <f32 as cpal::Sample>::from_sample(v);
        let last = self.frame_pos + 1 == self.ch;
        if last {
            // Average all channels into one mono signal so both speakers'
            // content is represented (was left-channel-only before).
            let mono = self.frame_acc / self.ch as f32;
            self.frame_acc = 0.0;
            self.ring[self.pos] = mono;
            self.pos = (self.pos + 1) % self.ring.len();
            self.acc_rms += mono * mono;
            self.acc_cnt += 1;
            self.since += 1;
            if self.since >= VIZ_HOP {
                self.analyze();
            }
        }
        self.frame_pos = (self.frame_pos + 1) % self.ch;
        Some(v)
    }
}

impl<S: rodio::Source> rodio::Source for VizTap<S>
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
        let r = self.inner.try_seek(pos);
        if r.is_ok() {
            self.ring.fill(0.0);
            self.pos = 0;
            self.since = 0;
            self.acc_rms = 0.0;
            self.acc_cnt = 0;
            self.frame_acc = 0.0;
            self.prev_rms = 0.0;
            self.smooth_rms = 0.0;
        }
        r
    }
}