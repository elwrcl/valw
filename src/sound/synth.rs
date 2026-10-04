//! The shutter sounds, synthesised: no samples, no licences.
//!
//! A two-hit mechanical shutter with a pitched body ("ka-chunk"), a metallic
//! clatter that grows with the combo, and on shots 6 and 7 a bell
//! arpeggio over a Shepard glide that seems to rise forever.

use std::f32::consts::TAU;

pub const RATE: u32 = 44_100;
const SPEEDS: [f32; 5] = [1.00, 1.12, 1.25, 1.40, 1.60];
const LOOP_SPEED: f32 = 1.80;
/// The shutter's second hit, after the first.
const GAP: f32 = 0.150;
/// How much clatter each of shots 1–5 carries (6 and 7 take the last).
const CLATTER: [f32; 5] = [0.0, 0.08, 0.15, 0.22, 0.30];
const CLATTER_HZ: f32 = 900.0;
/// The clatter's partials: (ratio to its fundamental, amplitude, decay s).
const CLATTER_PARTIALS: [(f32, f32, f32); 4] = [
    (1.0, 1.0, 0.060),
    (1.50, 0.7, 0.050),
    (4.40, 0.9, 0.030),
    (22.1, 0.25, 0.012),
];
/// Every sound is scaled to this short-term loudness (RMS).
const LOUDNESS: f32 = 0.30;
/// The tail of shots 6 and 7, relative to [`LOUDNESS`].
const TAIL_LOUDNESS: f32 = 0.8;
const TAIL_SECS: f32 = 0.30;
const ARPEGGIO_NOTES: usize = 3;
const ARPEGGIO_STEP: f32 = 0.075;
/// The Shepard tone: octave-spaced partials from BASE Hz under a bell
/// centred on CENTRE Hz (log scale).
const BASE: f32 = 55.0;
const PARTIALS: i32 = 9;
const CENTRE: f32 = 660.0;
const WIDTH_OCTAVES: f32 = 1.4;

/// A deterministic noise source (xorshift), so every build sounds the same.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn secs(n: usize) -> f32 {
    n as f32 / RATE as f32
}

fn samples(secs_long: f32) -> usize {
    (secs_long * RATE as f32) as usize
}

/// Adds `src` times `gain` into `dst` from `at` seconds, growing `dst`.
fn mix(dst: &mut Vec<f32>, src: &[f32], at: f32, gain: f32) {
    let start = samples(at);
    if dst.len() < start + src.len() {
        dst.resize(start + src.len(), 0.0);
    }
    for (d, s) in dst[start..].iter_mut().zip(src) {
        *d += s * gain;
    }
}

/// A damped resonance at `f` Hz with a small downward pitch blip and an
/// inharmonic overtone: the body of a hit.
fn thunk(f: f32, decay: f32) -> Vec<f32> {
    let (mut p1, mut p2) = (0.0f32, 0.0f32);
    (0..samples(decay * 7.0))
        .map(|i| {
            let t = secs(i);
            let env = (t / 0.0008).min(1.0) * (-t / decay).exp();
            let fi = f * (1.0 + 0.25 * (-t / 0.006).exp());
            p1 = (p1 + TAU * fi / RATE as f32) % TAU;
            p2 = (p2 + TAU * fi * 2.76 / RATE as f32) % TAU;
            env * (p1.sin() + 0.35 * p2.sin() * (-t / (decay * 0.4)).exp())
        })
        .collect()
}

/// A burst of high-passed noise; `bright` (0–1) tames the very top.
fn burst(decay: f32, seed: u32, bright: f32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let (mut low, mut high_prev, mut high) = (0.0f32, 0.0f32, 0.0f32);
    (0..samples(decay * 7.0))
        .map(|i| {
            let t = secs(i);
            low += bright * (noise.next() - low);
            high = 0.92 * (high + low - high_prev);
            high_prev = low;
            high * (t / 0.0005).min(1.0) * (-t / decay).exp()
        })
        .collect()
}

fn hit(f: f32, seed: u32, noise: f32) -> Vec<f32> {
    let mut s = thunk(f, 0.035);
    mix(&mut s, &burst(0.018, seed, 0.9), 0.0, noise);
    s
}

/// The mechanical "ka-chunk": two hits [`GAP`] apart, the second a fifth up.
pub fn shutter() -> Vec<f32> {
    let mut s = hit(260.0, 0x0123_4567, 0.55);
    mix(&mut s, &hit(390.0, 0x0765_4321, 0.65), GAP, 1.1);
    s
}

/// One metallic strike of the clatter at `f` Hz.
fn ping(f: f32) -> Vec<f32> {
    let mut phases = [0f32; CLATTER_PARTIALS.len()];
    (0..samples(0.35))
        .map(|i| {
            let t = secs(i);
            let mut v = 0.0;
            for (p, (ratio, amp, decay)) in phases.iter_mut().zip(CLATTER_PARTIALS) {
                *p = (*p + TAU * f * ratio / RATE as f32) % TAU;
                v += amp * p.sin() * (-t / decay).exp();
            }
            v * (t / 0.0004).min(1.0)
        })
        .collect()
}

/// A coin-like clatter: four quick, fading metallic strikes.
fn clatter(f: f32) -> Vec<f32> {
    let mut s = Vec::new();
    for (j, (at, amp)) in [(0.0, 1.0), (0.035, 0.7), (0.080, 0.5), (0.135, 0.35)]
        .into_iter()
        .enumerate()
    {
        mix(&mut s, &ping(f * (1.0 + 0.012 * j as f32)), at, amp);
        mix(
            &mut s,
            &burst(0.006, 0x9e37_79b9 + j as u32, 0.6),
            at,
            0.35 * amp,
        );
    }
    s
}

/// The shutter with clatter `level` (1–5): more of it, and two semitones
/// higher, each level.
fn shot(level: usize) -> Vec<f32> {
    let mut s = shutter();
    let amount = CLATTER[level - 1];
    if amount > 0.0 {
        let f = CLATTER_HZ * 2f32.powf((level - 1) as f32 * 2.0 / 12.0);
        mix(&mut s, &clatter(f), GAP, amount);
    }
    s
}

/// The loudest 20 ms of `s` (RMS): what the ear compares between shots.
pub fn loudness(s: &[f32]) -> f32 {
    let window = samples(0.020).min(s.len()).max(1);
    let mut sum: f32 = s[..window.min(s.len())].iter().map(|v| v * v).sum();
    let mut best = sum;
    for i in window..s.len() {
        sum += s[i] * s[i] - s[i - window] * s[i - window];
        best = best.max(sum);
    }
    (best.max(0.0) / window as f32).sqrt()
}

/// Scales `s` to `target` loudness through a soft limiter, keeps the peak
/// under 0.95 and fades the end.
fn finish(mut s: Vec<f32>, target: f32) -> Vec<f32> {
    let gain = target / loudness(&s).max(1e-9);
    for v in s.iter_mut() {
        *v = (*v * gain * 1.2).tanh() / 1.2;
    }
    let max = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if max > 0.95 {
        for v in s.iter_mut() {
            *v *= 0.95 / max;
        }
    }
    fade_out(&mut s, 0.010);
    s
}

/// `samples` played `factor` times faster (shorter and higher).
pub fn speed(samples: &[f32], factor: f32) -> Vec<f32> {
    let len = (samples.len() as f32 / factor).ceil() as usize;
    (0..len)
        .map(|i| {
            let pos = i as f32 * factor;
            let j = pos as usize;
            let frac = pos - j as f32;
            let a = samples.get(j).copied().unwrap_or(0.0);
            let b = samples.get(j + 1).copied().unwrap_or(0.0);
            a + (b - a) * frac
        })
        .collect()
}

/// The Shepard tone's partials `(frequency, amplitude)` at `position`
/// octaves (0..1) along the glide. Amplitude depends only on frequency, so
/// position 1 sounds like position 0.
pub fn partials(position: f32) -> Vec<(f32, f32)> {
    (-1..PARTIALS)
        .map(|k| {
            let f = BASE * 2f32.powf(k as f32 + position);
            let d = (f / CENTRE).log2() / WIDTH_OCTAVES;
            (f, (-0.5 * d * d).exp())
        })
        .collect()
}

/// A Shepard glide from `from` to `to` octaves over `secs` seconds.
pub fn glide(from: f32, to: f32, secs_long: f32) -> Vec<f32> {
    let len = (secs_long * RATE as f32) as usize;
    let count = partials(0.0).len();
    let mut phases = vec![0f32; count];
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let position = from + (to - from) * i as f32 / len as f32;
        let mut v = 0.0;
        for (p, (f, a)) in phases.iter_mut().zip(partials(position)) {
            *p = (*p + TAU * f / RATE as f32) % TAU;
            v += a * p.sin();
        }
        out.push(v);
    }
    fade_in(&mut out, 0.010);
    fade_out(&mut out, 0.040);
    normalise(&mut out, 0.5);
    out
}

/// One bell note of the Shepard scale at `position` octaves: octave
/// partials with a short metallic overtone.
fn bell(position: f32) -> Vec<f32> {
    let partials = partials(position);
    let mut phases = vec![(0f32, 0f32); partials.len()];
    (0..samples(0.16))
        .map(|i| {
            let t = secs(i);
            let mut v = 0.0;
            for ((p, q), (f, a)) in phases.iter_mut().zip(&partials) {
                *p = (*p + TAU * f / RATE as f32) % TAU;
                *q = (*q + TAU * f * 4.4 / RATE as f32) % TAU;
                v += a * (p.sin() + 0.25 * q.sin() * (-t / 0.015).exp());
            }
            v * (t / 0.0015).min(1.0) * (-t / 0.05).exp()
        })
        .collect()
}

/// [`ARPEGGIO_NOTES`] bell notes climbing from `from` towards `to`
/// octaves, over a quiet Shepard glide.
fn tail(from: f32, to: f32) -> Vec<f32> {
    let mut notes = Vec::new();
    for j in 0..ARPEGGIO_NOTES {
        let position = from + (to - from) * j as f32 / ARPEGGIO_NOTES as f32;
        mix(&mut notes, &bell(position), j as f32 * ARPEGGIO_STEP, 1.0);
    }
    let pad = glide(from, to, TAIL_SECS);
    let gain = 0.45 / loudness(&pad);
    let mut s: Vec<f32> = pad.iter().map(|v| v * gain).collect();
    mix(&mut s, &notes, 0.0, 1.0 / loudness(&notes));
    s
}

/// The sound for shot `n` of a combo (1–7).
pub fn sound(n: u8) -> Vec<f32> {
    match n {
        1..=5 => finish(speed(&shot(n as usize), SPEEDS[n as usize - 1]), LOUDNESS),
        6 | 7 => {
            let mut s = finish(speed(&shot(5), LOOP_SPEED), LOUDNESS);
            let half = if n == 6 { (0.0, 0.5) } else { (0.5, 1.0) };
            let t = tail(half.0, half.1);
            let gain = LOUDNESS * TAIL_LOUDNESS / loudness(&t);
            s.extend(t.iter().map(|v| v * gain));
            s
        }
        _ => sound(1),
    }
}

/// 16-bit mono PCM WAV.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

fn fade_in(s: &mut [f32], secs_long: f32) {
    let n = ((secs_long * RATE as f32) as usize).min(s.len());
    for (i, v) in s[..n].iter_mut().enumerate() {
        *v *= i as f32 / n as f32;
    }
}

fn fade_out(s: &mut [f32], secs_long: f32) {
    let n = ((secs_long * RATE as f32) as usize).min(s.len());
    let len = s.len();
    for (i, v) in s[len - n..].iter_mut().enumerate() {
        *v *= 1.0 - (i + 1) as f32 / n as f32;
    }
}

fn normalise(s: &mut [f32], peak: f32) {
    let max = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if max > 0.0 {
        for v in s.iter_mut() {
            *v *= peak / max;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    fn rms(s: &[f32], from_ms: f32, to_ms: f32) -> f32 {
        let at = |ms: f32| ((ms / 1000.0 * RATE as f32) as usize).min(s.len());
        let seg = &s[at(from_ms)..at(to_ms)];
        (seg.iter().map(|v| v * v).sum::<f32>() / seg.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_shutter_is_two_hits_and_never_clips() {
        let s = shutter();
        let ms = s.len() as f32 * 1000.0 / RATE as f32;
        assert!((300.0..=450.0).contains(&ms), "{ms} ms");
        let (first, between, second) = (
            rms(&s, 0.0, 20.0),
            rms(&s, 115.0, 140.0),
            rms(&s, 150.0, 170.0),
        );
        assert!(first > 3.0 * between, "{first} vs {between}");
        assert!(
            second > 3.0 * between,
            "a second hit at 150 ms: {second} vs {between}"
        );
        assert!(s.last().unwrap().abs() < 0.01, "ends quietly (no pop)");
    }

    #[test]
    fn every_shot_is_about_as_loud() {
        let db: Vec<f32> = (1..=7)
            .map(|n| 20.0 * loudness(&sound(n)).log10())
            .collect();
        let (lo, hi) = db
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(hi - lo < 1.5, "{db:?}");
    }

    #[test]
    fn every_shot_ends_quietly() {
        for n in 1..=7 {
            assert!(sound(n).last().unwrap().abs() < 0.01, "sound {n} pops");
        }
    }

    #[test]
    fn faster_is_shorter() {
        let s = shutter();
        assert_eq!(speed(&s, 1.6).len(), (s.len() as f32 / 1.6).ceil() as usize);
    }

    #[test]
    fn sounds_escalate_then_loop() {
        assert!(SPEEDS.windows(2).all(|w| w[1] > w[0]) && LOOP_SPEED > SPEEDS[4]);
        for n in 1..=5u8 {
            let s = sound(n);
            let at = GAP * 1000.0 / SPEEDS[n as usize - 1];
            let (before, hit) = (rms(&s, at - 25.0, at - 5.0), rms(&s, at, at + 15.0));
            assert!(
                hit > 2.0 * before,
                "shot {n}: second hit at {at} ms ({hit} vs {before})"
            );
        }
        assert!(sound(6).len() > sound(5).len(), "6 adds the glide");
        assert_eq!(sound(6).len(), sound(7).len());
        for n in 1..=7 {
            assert!(peak(&sound(n)) <= 0.95, "sound {n} clips");
        }
    }

    #[test]
    fn the_glide_returns_to_its_start_an_octave_up() {
        let start = partials(0.0);
        let end = partials(1.0);
        for (f, a) in start.iter().filter(|p| p.1 > 0.01) {
            let twin = end
                .iter()
                .find(|p| (p.0 - f).abs() < 0.01)
                .expect("same frequency an octave later");
            assert!((twin.1 - a).abs() < 1e-3, "{f} Hz: {a} vs {}", twin.1);
        }
    }

    #[test]
    fn wav_header() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), RATE);
        assert_eq!(w.len(), 44 + 3 * 2);
        assert_eq!(i16::from_le_bytes(w[46..48].try_into().unwrap()), 16384);
    }
}
