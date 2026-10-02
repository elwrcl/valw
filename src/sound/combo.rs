//! Which shot of a combo this is, shared between `valw` runs.

use std::path::Path;

use anyhow::Result;

/// The number for a shot at `now_ms`, given the previous shot.
pub fn next(prev: Option<(u8, u64)>, now_ms: u64, reset_ms: u64) -> u8 {
    match prev {
        Some((n, at)) if now_ms >= at && now_ms - at < reset_ms => match n {
            7 => 6,
            n => n + 1,
        },
        _ => 1,
    }
}

/// `(number, unix ms)` of the last shot, if the file makes sense.
pub fn load(path: &Path) -> Option<(u8, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let n: u8 = parts.next()?.parse().ok()?;
    let at: u64 = parts.next()?.parse().ok()?;
    (1..=7).contains(&n).then_some((n, at))
}

pub fn save(path: &Path, n: u8, at_ms: u64) -> Result<()> {
    crate::output::write_atomic(path, format!("{n} {at_ms}\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_up_then_alternates_six_and_seven() {
        let mut prev = None;
        let mut seen = Vec::new();
        for i in 0..11u64 {
            let n = next(prev, i * 1000, 5000);
            seen.push(n);
            prev = Some((n, i * 1000));
        }
        assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7, 6, 7, 6, 7]);
    }

    #[test]
    fn a_pause_resets() {
        assert_eq!(next(Some((4, 1000)), 7000, 5000), 1);
        assert_eq!(next(Some((4, 1000)), 5999, 5000), 5);
        assert_eq!(next(None, 0, 5000), 1);
        assert_eq!(
            next(Some((3, 9000)), 1000, 5000),
            1,
            "a clock going backwards"
        );
    }

    #[test]
    fn state_file_round_trip_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw-combo");
        assert_eq!(load(&path), None);
        save(&path, 6, 123456).unwrap();
        assert_eq!(load(&path), Some((6, 123456)));
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "9 5").unwrap();
        assert_eq!(load(&path), None, "out of range");
    }
}
