//! Physiological signal processing utilities for BIOPAC recordings.
//!
//! This module provides pure-Rust algorithms for common biosignal analysis
//! tasks that arise when working with `.acq` recordings: detecting trigger
//! pulses, finding ECG R-peaks, finding PPG feet, and computing pulse-transit
//! time (PTT).
//!
//! All functions operate on plain `&[f64]` slices and return owned `Vec`s or
//! scalars — no heap allocator required beyond the outputs themselves.
//!
//! # Feature gate
//!
//! This module is enabled by the `physio` feature (off by default).
//!
//! # Examples
//!
//! ```rust,ignore
//! use biodream::signals::{detect_r_peaks, detect_ppg_feet, beat_ptt, median_ptt};
//!
//! // After loading your ECG and PPG channels …
//! let ecg = lazy.load_channel_containing("ecg")?.scaled_samples();
//! let ppg = lazy.load_channel_containing("ppg finger")?.scaled_samples();
//! let fs = 1000.0; // samples/second
//!
//! let r_peaks = detect_r_peaks(&ecg, fs);
//! let ppg_feet = detect_ppg_feet(&ppg, fs);
//! let ptt_beats = beat_ptt(&r_peaks, &ppg_feet, fs, 30.0, 380.0);
//! let ptt_ms = median_ptt(&r_peaks, &ppg_feet, fs);
//! println!("Median PTT: {:?} ms  ({} beats)", ptt_ms, ptt_beats.len());
//! ```

extern crate alloc;
use alloc::vec::Vec;

// --------------------------------------------------------------------------- //
// Internal helpers                                                              //
// --------------------------------------------------------------------------- //

/// Sliding-window moving average (causal, forward pass).
fn moving_average(x: &[f64], win: usize) -> Vec<f64> {
    let win = win.max(1);
    let n = x.len();
    let mut out = Vec::with_capacity(n);
    let mut sum = 0.0_f64;
    for (i, &v) in x.iter().enumerate() {
        sum += v;
        if i >= win
            && let Some(&prev) = x.get(i - win)
        {
            sum -= prev;
        }
        #[expect(clippy::cast_precision_loss)]
        let divisor = win.min(i + 1) as f64;
        out.push(sum / divisor);
    }
    out
}

/// Derivative kernel from Pan-Tompkins (5-point weighted first difference).
///
/// `d[i]` approximates the derivative of `x` at index `i + 2`.
/// Output length is `x.len().saturating_sub(4)`.
#[expect(clippy::many_single_char_names)]
fn derivative5(x: &[f64]) -> Vec<f64> {
    if x.len() < 5 {
        return Vec::new();
    }
    x.windows(5)
        .map(|w| match w {
            [a, b, _, d, e] => (-2.0_f64 * *a - *b + *d + 2.0 * *e) / 8.0,
            _ => 0.0,
        })
        .collect()
}

/// Find local maxima above a height threshold with a minimum inter-peak distance.
///
/// When two peaks are closer than `min_dist` the taller one is kept.
fn local_maxima(signal: &[f64], min_dist: usize, min_height: f64) -> Vec<usize> {
    let n = signal.len();
    if n < 3 {
        return Vec::new();
    }
    let mut peaks: Vec<usize> = Vec::new();
    for i in 1..n - 1 {
        let (Some(&curr), Some(&prev), Some(&next)) =
            (signal.get(i), signal.get(i - 1), signal.get(i + 1))
        else {
            continue;
        };
        if curr <= min_height {
            continue;
        }
        if curr <= prev || curr < next {
            continue;
        }
        match peaks.last_mut() {
            Some(last) if i - *last < min_dist => {
                if let Some(&last_val) = signal.get(*last)
                    && curr > last_val
                {
                    *last = i;
                }
            }
            _ => {
                peaks.push(i);
            }
        }
    }
    peaks
}

/// Find local minima below a depth threshold with a minimum inter-valley distance.
fn local_minima(signal: &[f64], min_dist: usize, max_height: f64) -> Vec<usize> {
    let n = signal.len();
    if n < 3 {
        return Vec::new();
    }
    let mut valleys: Vec<usize> = Vec::new();
    for i in 1..n - 1 {
        let (Some(&curr), Some(&prev), Some(&next)) =
            (signal.get(i), signal.get(i - 1), signal.get(i + 1))
        else {
            continue;
        };
        if curr >= max_height {
            continue;
        }
        if curr >= prev || curr > next {
            continue;
        }
        match valleys.last_mut() {
            Some(last) if i - *last < min_dist => {
                if let Some(&last_val) = signal.get(*last)
                    && curr < last_val
                {
                    *last = i;
                }
            }
            _ => {
                valleys.push(i);
            }
        }
    }
    valleys
}

/// Compute the p-th percentile (0–100) of a sorted slice.
///
/// Returns `None` if the slice is empty.  Accepts a pre-sorted slice for
/// efficiency when the caller already has sorted data.
fn percentile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    if n == 1 {
        return sorted.first().copied();
    }
    #[expect(clippy::cast_precision_loss)]
    let idx = p / 100.0 * (n - 1) as f64;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = idx as usize;
    let hi = (lo + 1).min(n - 1);
    #[expect(clippy::cast_precision_loss)]
    let frac = idx - lo as f64;
    let lo_val = sorted.get(lo).copied()?;
    let hi_val = sorted.get(hi).copied()?;
    Some(lo_val * (1.0 - frac) + hi_val * frac)
}

// --------------------------------------------------------------------------- //
// Public API                                                                    //
// --------------------------------------------------------------------------- //

/// Detect rising edges in a digital (square-wave) signal and return their
/// sample indices.
///
/// A rising edge is defined as the first sample whose value is ≥ `threshold`
/// after one or more samples below `threshold`.
///
/// Typical use: finding the start and end of the measurement window in a BIOPAC
/// Sync channel.
///
/// # Example
///
/// ```rust,ignore
/// let sync = lazy.load_channel_containing("sync")?.scaled_samples();
/// let edges = biodream::signals::rising_edges(&sync, 0.5);
/// if edges.len() >= 2 {
///     let (window_start, window_end) = (edges[0], edges[1]);
/// }
/// ```
pub fn rising_edges(signal: &[f64], threshold: f64) -> Vec<usize> {
    if signal.len() < 2 {
        return Vec::new();
    }
    signal
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| match w {
            [prev, curr] if *prev < threshold && *curr >= threshold => Some(i + 1),
            _ => None,
        })
        .collect()
}

/// Detect falling edges in a digital (square-wave) signal.
///
/// Symmetric to [`rising_edges`]: returns the first sample index that drops
/// below `threshold`.
pub fn falling_edges(signal: &[f64], threshold: f64) -> Vec<usize> {
    if signal.len() < 2 {
        return Vec::new();
    }
    signal
        .windows(2)
        .enumerate()
        .filter_map(|(i, w)| match w {
            [prev, curr] if *prev >= threshold && *curr < threshold => Some(i + 1),
            _ => None,
        })
        .collect()
}

/// Extract the indices `[start, end)` of the first measurement window bounded
/// by two rising edges in a Sync channel.
///
/// Returns `(0, signal.len())` when fewer than two rising edges are found so
/// that callers always get a valid range.
pub fn sync_window(sync: &[f64], threshold: f64) -> (usize, usize) {
    let edges = rising_edges(sync, threshold);
    match edges.as_slice() {
        [a, b, ..] => (*a, *b),
        [a] => (*a, sync.len()),
        [] => (0, sync.len()),
    }
}

/// Detect ECG QRS R-peaks using a Pan-Tompkins–inspired algorithm.
///
/// Steps:
/// 1. 5-point derivative (approximates a bandpass 5–25 Hz HPF)
/// 2. Squaring (emphasises the QRS complex)
/// 3. 150 ms moving-window integration
/// 4. Local maxima search with 300 ms minimum inter-peak distance and an
///    adaptive 25th-percentile height threshold
///
/// `fs` is the sampling rate in Hz.  Returns sample indices into the **original**
/// `ecg` slice (not the derivative output).
///
/// # Panics
///
/// Never panics.
pub fn detect_r_peaks(ecg: &[f64], fs: f64) -> Vec<usize> {
    if ecg.len() < 5 || fs <= 0.0 {
        return Vec::new();
    }
    // Pan-Tompkins derivative (output is shifted by +2 samples).
    let diff = derivative5(ecg);
    let squared: Vec<f64> = diff.iter().map(|&x| x * x).collect();

    // 150 ms integration window.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let win = (0.150 * fs).round() as usize + 1;
    let integrated = moving_average(&squared, win);

    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_dist = (0.300 * fs).round() as usize;

    // Adaptive threshold: 25th percentile of the integrated signal.
    let mut sorted = integrated.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let threshold = percentile_sorted(&sorted, 25.0).unwrap_or(0.0) * 0.25
        + percentile_sorted(&sorted, 75.0).unwrap_or(0.0) * 0.05;

    local_maxima(&integrated, min_dist, threshold)
        .into_iter()
        // Shift back to account for the 2-sample derivative offset.
        .map(|i| i.saturating_add(2).min(ecg.len() - 1))
        .collect()
}

/// Detect PPG pulse onset (foot) points.
///
/// The foot is the local minimum immediately before each systolic peak.
/// Algorithm:
/// 1. 100 ms moving-average smoothing (removes high-frequency noise)
/// 2. Subtract a 1 s moving average to remove slow drift
/// 3. Find local minima with 300 ms minimum inter-valley distance and a
///    threshold set to the 15th percentile of the signal
///
/// `fs` is the sampling rate in Hz.  Returns sample indices.
pub fn detect_ppg_feet(ppg: &[f64], fs: f64) -> Vec<usize> {
    if ppg.len() < 5 || fs <= 0.0 {
        return Vec::new();
    }
    // Short smoothing window (~100 ms)
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let short_win = (0.100 * fs).round() as usize + 1;
    let smooth = moving_average(ppg, short_win);

    // Remove slow baseline drift with a 1 s window moving average.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let long_win = (1.0 * fs).round() as usize + 1;
    let baseline = moving_average(&smooth, long_win);
    let detrended: Vec<f64> = smooth
        .iter()
        .zip(baseline.iter())
        .map(|(&s, &b)| s - b)
        .collect();

    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_dist = (0.300 * fs).round() as usize;

    // Threshold: keep only minima below the 15th percentile.
    let mut sorted = detrended.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let threshold = percentile_sorted(&sorted, 15.0).unwrap_or(f64::NEG_INFINITY);

    local_minima(&detrended, min_dist, threshold)
}

/// Compute per-beat pulse-transit time (PTT) in milliseconds.
///
/// For each R-peak, finds the nearest PPG foot occurring between `min_ms` and
/// `max_ms` after the R-peak.  Returns one PTT value per matched beat in the
/// same order as the input R-peaks.
///
/// Beats without a matching PPG foot in the search window are silently skipped.
///
/// `fs` is the common sampling rate of both index arrays.  If ECG and PPG were
/// recorded at different rates, resample the peak indices to a common rate
/// before calling this function.
///
/// Typical search window: `min_ms = 30.0`, `max_ms = 380.0`.
pub fn beat_ptt(
    r_peaks: &[usize],
    ppg_feet: &[usize],
    fs: f64,
    min_ms: f64,
    max_ms: f64,
) -> Vec<f64> {
    if r_peaks.is_empty() || ppg_feet.is_empty() || fs <= 0.0 {
        return Vec::new();
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let min_offset = (min_ms * fs / 1000.0).round() as usize;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_offset = (max_ms * fs / 1000.0).round() as usize;

    r_peaks
        .iter()
        .filter_map(|&r| {
            let lo = r.saturating_add(min_offset);
            let hi = r.saturating_add(max_offset);
            let candidate = ppg_feet.iter().find(|&&f| f >= lo && f <= hi)?;
            let diff = (*candidate).saturating_sub(r);
            #[expect(clippy::cast_precision_loss)]
            let ptt = diff as f64 / fs * 1000.0;
            Some(ptt)
        })
        .collect()
}

/// Compute the median per-beat PTT (ms) between ECG R-peaks and PPG feet.
///
/// Convenience wrapper around [`beat_ptt`] using a default search window of
/// [30 ms, 380 ms].  Returns `None` if no beats are matched.
pub fn median_ptt(r_peaks: &[usize], ppg_feet: &[usize], fs: f64) -> Option<f64> {
    let mut ptts = beat_ptt(r_peaks, ppg_feet, fs, 30.0, 380.0);
    if ptts.is_empty() {
        return None;
    }
    ptts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = ptts.len();
    if n % 2 == 1 {
        ptts.get(n / 2).copied()
    } else {
        let lo = ptts.get(n / 2 - 1)?;
        let hi = ptts.get(n / 2)?;
        Some((lo + hi) / 2.0)
    }
}

/// Compute heart rate in BPM from R-peak sample indices.
///
/// Returns `None` if fewer than two R-peaks are present.
pub fn heart_rate_bpm(r_peaks: &[usize], fs: f64) -> Option<f64> {
    if r_peaks.len() < 2 || fs <= 0.0 {
        return None;
    }
    let rr_intervals: Vec<f64> = r_peaks
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => {
                #[expect(clippy::cast_precision_loss)]
                let diff = b.checked_sub(*a)? as f64;
                let rr_s = diff / fs;
                // Reject physiologically impossible RR intervals
                if (0.25..=2.0).contains(&rr_s) {
                    Some(rr_s)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    if rr_intervals.is_empty() {
        return None;
    }
    #[expect(clippy::cast_precision_loss)]
    let mean_rr = rr_intervals.iter().sum::<f64>() / rr_intervals.len() as f64;
    Some(60.0 / mean_rr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn rising_edges_basic() {
        let sig = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        let edges = rising_edges(&sig, 0.5);
        assert_eq!(edges, [2, 5]);
    }

    #[test]
    fn falling_edges_basic() {
        let sig = [0.0, 1.0, 1.0, 0.0, 1.0, 0.0];
        let edges = falling_edges(&sig, 0.5);
        assert_eq!(edges, [3, 5]);
    }

    #[test]
    fn sync_window_two_pulses() {
        let mut sig = vec![0.0_f64; 1000];
        if let Some(v) = sig.get_mut(100) {
            *v = 1.0;
        }
        if let Some(v) = sig.get_mut(101) {
            *v = 1.0;
        }
        if let Some(v) = sig.get_mut(800) {
            *v = 1.0;
        }
        if let Some(v) = sig.get_mut(801) {
            *v = 1.0;
        }
        let (start, end) = sync_window(&sig, 0.5);
        assert_eq!(start, 100);
        assert_eq!(end, 800);
    }

    #[test]
    fn sync_window_no_pulses_returns_full_range() {
        let sig = vec![0.0_f64; 500];
        let (start, end) = sync_window(&sig, 0.5);
        assert_eq!(start, 0);
        assert_eq!(end, 500);
    }

    #[test]
    fn detect_r_peaks_synthetic_ecg() {
        // Synthesise a 5-second ECG at 1000 Hz with R-peaks every 1000 ms (60 BPM).
        let fs = 1000.0_f64;
        let n = 5000_usize;
        let mut ecg = vec![0.0_f64; n];
        let peak_positions = [500_usize, 1500, 2500, 3500, 4500];
        for &p in &peak_positions {
            // Narrow Gaussian-ish spike
            for di in 0_usize..30 {
                #[expect(clippy::cast_precision_loss)]
                let dist = di as f64;
                let val = (-0.5 * (dist / 5.0).powi(2)).exp();
                if p + di < n
                    && let Some(v) = ecg.get_mut(p + di)
                {
                    *v += val;
                }
                if p >= di
                    && let Some(v) = ecg.get_mut(p - di)
                {
                    *v += val;
                }
            }
        }
        let peaks = detect_r_peaks(&ecg, fs);
        // Should find all 5 R-peaks.  The Pan-Tompkins integration window
        // introduces a causal delay of roughly half the window length (~75
        // samples at 1000 Hz), so we accept ±100 samples.
        assert!(
            peaks.len() >= 4,
            "expected ≥4 R-peaks, got {}: {:?}",
            peaks.len(),
            peaks
        );
        for (&expected, &found) in peak_positions.iter().zip(peaks.iter()) {
            let diff = found.abs_diff(expected);
            assert!(
                diff <= 100,
                "R-peak offset too large: expected ~{expected}, got {found}"
            );
        }
    }

    #[test]
    fn beat_ptt_synthetic() {
        // R-peaks at 0, 1000, 2000; PPG feet at 150, 1150, 2150 — PTT = 150 ms
        let r = [0_usize, 1000, 2000];
        let f = [150_usize, 1150, 2150];
        let ptts = beat_ptt(&r, &f, 1000.0, 30.0, 380.0);
        assert_eq!(ptts.len(), 3);
        for ptt in &ptts {
            let diff = (ptt - 150.0).abs();
            assert!(diff < 0.1, "unexpected PTT: {ptt}");
        }
    }

    #[test]
    fn median_ptt_returns_median() {
        let r = [0_usize, 1000, 2000, 3000];
        let f = [100_usize, 1120, 2080, 3100];
        // PTTs: 100, 120, 80, 100 ms  →  sorted [80, 100, 100, 120]  →  median = 100
        let med = median_ptt(&r, &f, 1000.0);
        let m = med.unwrap_or(f64::NAN);
        assert!(m.is_finite(), "expected Some median, got None");
        assert!((m - 100.0).abs() < 1.0, "expected median ~100ms, got {m}");
    }

    #[test]
    fn heart_rate_bpm_basic() {
        // R-peaks every 1000 samples at 1000 Hz = 1.0 s RR = 60 BPM
        let peaks: Vec<usize> = (0..6).map(|i| i * 1000).collect();
        let hr = heart_rate_bpm(&peaks, 1000.0).unwrap_or(f64::NAN);
        assert!(
            hr.is_finite(),
            "expected heart_rate_bpm to return Some, got None"
        );
        assert!((hr - 60.0).abs() < 0.5, "expected 60 BPM, got {hr}");
    }
}
