//! The shutter sounds, synthesised: no samples, no licences.

use std::f32::consts::TAU;

pub const RATE: u32 = 44_100;
const SPEEDS: [f32; 5] = [1.00, 1.12, 1.25, 1.40, 1.60];
const LOOP_SPEED: f32 = 1.80;
const GLIDE_SECS: f32 = 0.4;
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

/// A two-part mechanical "tak-shk", about 150 ms.
pub fn shutter() -> Vec<f32> {
    let len = (0.150 * RATE as f32) as usize;
    let mut noise = Noise(0x1234_5678);
    let (mut low, mut high_prev, mut high) = (0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = secs(i);
        let n = noise.next();
        // Click: bright, 0.5 ms attack, ~6 ms decay.
        let click = n * (t / 0.0005).min(1.0) * (-t / 0.006).exp();
        // "shk": band-passed noise from 60 ms, ~30 ms decay, softer.
        low += 0.25 * (n - low);
        high = 0.9 * (high + low - high_prev);
        high_prev = low;
        let u = t - 0.060;
        let shk = if u >= 0.0 {
            high * 0.9 * (u / 0.002).min(1.0) * (-u / 0.03).exp()
        } else {
            0.0
        };
        out.push(click * 0.8 + shk);
    }
    fade_out(&mut out, 0.010);
    normalise(&mut out, 0.9);
    out
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

/// The sound for shot `n` of a combo (1–7).
pub fn sound(n: u8) -> Vec<f32> {
    match n {
        1..=5 => speed(&shutter(), SPEEDS[n as usize - 1]),
        6 | 7 => {
            let mut s = speed(&shutter(), LOOP_SPEED);
            let half = if n == 6 { (0.0, 0.5) } else { (0.5, 1.0) };
            s.extend(glide(half.0, half.1, GLIDE_SECS));
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

    #[test]
    fn the_shutter_is_short_and_never_clips() {
        let s = shutter();
        let ms = s.len() as f32 * 1000.0 / RATE as f32;
        assert!((140.0..=160.0).contains(&ms), "{ms} ms");
        assert!(peak(&s) <= 0.95 && peak(&s) > 0.3);
        assert!(s.last().unwrap().abs() < 0.01, "ends quietly (no pop)");
    }

    #[test]
    fn faster_is_shorter() {
        let s = shutter();
        assert_eq!(speed(&s, 1.6).len(), (s.len() as f32 / 1.6).ceil() as usize);
    }

    #[test]
    fn sounds_escalate_then_loop() {
        let lens: Vec<usize> = (1..=5).map(|n| sound(n).len()).collect();
        assert!(lens.windows(2).all(|w| w[1] < w[0]), "{lens:?}");
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
