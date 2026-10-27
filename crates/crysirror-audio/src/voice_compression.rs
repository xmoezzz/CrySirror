// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded offline voice compression. The named Siglus JITAN numeric profile
//! follows the pinned public algorithm's PCM rules; no VM or playback engine.
use anyhow::{Result, ensure};
use crysirror_tasks::Cancellation;
pub const MAX_INPUT_FRAMES: usize = 4_194_304;
pub const MAX_OUTPUT_FRAMES: usize = 8_388_608;
pub struct Pcm16 {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}
fn bounded(n: i32) -> i16 {
    n.clamp(-32760, 32760) as i16
}
fn direction(samples: &[i16], at: usize, end: usize) -> i32 {
    if at >= end || at >= samples.len() {
        return 0;
    }
    let mut score = 0;
    for &s in samples[..at].iter().rev().take(10) {
        score += i32::from(s > samples[at]) - i32::from(s < samples[at]);
        if score >= 3 {
            return 1;
        }
        if score <= -3 {
            return -1;
        }
    }
    0
}
fn candidate(
    samples: &[i16],
    at: usize,
    count: usize,
    step: isize,
    wanted: i32,
    target: i32,
) -> Option<usize> {
    let (mut best, mut distance, mut preferred) = (None, 999999, false);
    for offset in 0..count {
        let pos = at as isize + step * offset as isize;
        if pos < 0 || pos as usize >= samples.len() {
            break;
        }
        let pos = pos as usize;
        let value = i32::from(samples[pos]);
        if wanted == 0 || direction(samples, pos, samples.len()) == wanted {
            let good = wanted != 0
                && (if wanted == 1 {
                    value <= target
                } else {
                    value >= target
                });
            let delta = (target - value).abs();
            if good {
                if !preferred || delta < distance {
                    best = Some(offset);
                    distance = delta;
                    preferred = true
                }
            } else if !preferred && delta < distance {
                best = Some(offset);
                distance = delta;
            }
        }
        if best.is_some() && distance <= 100 {
            break;
        }
    }
    best
}
fn align(input: &[i16], at: usize, out: &[i16], last: usize, base: usize, bound: usize) -> usize {
    let (trend, target) = if last < bound {
        (direction(out, last, bound), i32::from(out[last]))
    } else {
        (0, 0)
    };
    for phase in [trend, 0] {
        for step in [-1isize, 1] {
            if let Some(n) = candidate(input, at, base / 4, step, phase, target) {
                return if step < 0 {
                    at.saturating_sub(n)
                } else {
                    (at + n).min(input.len())
                };
            }
        }
    }
    at
}
fn segment(input: &[i16], at: usize, base: usize) -> usize {
    if input.len() - at <= base {
        return input.len() - at;
    }
    let mut end = at + base - 1;
    let mut minimum = i32::from(input[end]).abs();
    for pos in (at..end).rev().take(10) {
        let v = i32::from(input[pos]).abs();
        if v < minimum {
            minimum = v;
            end = pos;
        }
    }
    if input[end] == 0 {
        return end + 1 - at;
    }
    let length = end + 1 - at;
    let negative = input[end] < 0;
    let limit = length / 4;
    let scan = |step: isize| {
        let mut best = i32::from(input[end]).abs();
        let mut offset = 0;
        for n in 1..limit.max(2) {
            let p = end as isize + step * n as isize;
            if p < at as isize || p >= input.len() as isize {
                break;
            }
            let s = input[p as usize];
            if (s < 0) != negative {
                break;
            }
            let m = i32::from(s).abs();
            if m < best {
                best = m;
                offset = n;
                if m == 0 {
                    break;
                }
            }
        }
        (best, offset)
    };
    let f = scan(1);
    let b = scan(-1);
    if f.0 <= b.0 {
        (length + f.1).min(input.len() - at)
    } else {
        length - b.1
    }
}
/// Compress a mono/stereo PCM16 cue at 100..400 percent. 100 is exact identity;
/// other rates fold stereo by clamped sum, retaining rate and mono sample units.
/// The legacy profile silences cues shorter than 100 ms and fades the last 100
/// output samples; it is deliberately distinct from changing playback rate.
pub fn siglus_jitan(input: &[i16], channels: u16, rate: u32, percent: u16) -> Result<Pcm16> {
    siglus_jitan_cancellable(input, channels, rate, percent, &Cancellation::default())
}
pub fn siglus_jitan_cancellable(
    input: &[i16],
    channels: u16,
    rate: u32,
    percent: u16,
    cancel: &Cancellation,
) -> Result<Pcm16> {
    ensure!(
        (1..=2).contains(&channels)
            && (8000..=192000).contains(&rate)
            && (100..=400).contains(&percent),
        "voice compression parameters"
    );
    ensure!(
        !input.is_empty()
            && input.len() % channels as usize == 0
            && input.len() / channels as usize <= MAX_INPUT_FRAMES,
        "voice compression input budget/alignment"
    );
    cancel.check()?;
    if percent == 100 {
        return Ok(Pcm16 {
            rate,
            channels,
            samples: input.to_vec(),
        });
    }
    let mono = if channels == 1 {
        input.to_vec()
    } else {
        input
            .chunks_exact(2)
            .map(|s| (i32::from(s[0]) + i32::from(s[1])).clamp(-30000, 30000) as i16)
            .collect()
    };
    if mono.len() < (rate / 10) as usize {
        return Ok(Pcm16 {
            rate,
            channels: 1,
            samples: vec![0; mono.len()],
        });
    }
    let base = rate as usize / 80;
    let radius = rate as usize / 110;
    let skip = base * usize::from(percent - 100) / 100;
    let mut output = vec![0i16; mono.len() + rate as usize];
    let mut seam = vec![0i16; radius * 2];
    let (mut source, mut written, mut previous_copy, mut previous_after) =
        (0usize, 0usize, 0usize, 0usize);
    while source < mono.len() {
        cancel.check()?;
        let prior_source = source;
        if written > 0 {
            source = align(&mono, source, &output, written - 1, base, mono.len());
        }
        if source >= mono.len() {
            break;
        }
        let copy = segment(&mono, source, base);
        let before = radius.min(source);
        seam[radius - before..radius].copy_from_slice(&mono[source - before..source]);
        ensure!(
            written + copy <= MAX_OUTPUT_FRAMES,
            "voice compression output budget"
        );
        if written + copy > output.len() {
            output.resize(written + copy, 0);
        }
        output[written..written + copy].copy_from_slice(&mono[source..source + copy]);
        let delta = if written > 0 && written < mono.len() {
            (i32::from(output[written]) - i32::from(output[written - 1])) >> 1
        } else {
            0
        };
        // Preserve native integer truncation and asymmetric early-zero stops.
        for (count, step) in [(previous_copy, -1isize), (copy, 1)] {
            for n in 0..count {
                let at = if step < 0 {
                    written as isize - 1 - n as isize
                } else {
                    written as isize + n as isize
                };
                if at < 0 || at >= mono.len() as isize {
                    break;
                }
                let adjustment = ((delta as f64 / count as f64) * (count - n) as f64) as i32;
                if adjustment == 0 {
                    break;
                }
                output[at as usize] =
                    bounded(i32::from(output[at as usize]) - step as i32 * adjustment);
            }
        }
        previous_copy = copy;
        if delta != 0 {
            for (count, begin, sign) in [(before, radius - before, -1), (previous_after, radius, 1)]
            {
                for sample in &mut seam[begin..begin + count] {
                    *sample = bounded(i32::from(*sample) + sign * delta);
                }
            }
        }
        let left = before.min(written);
        let right = previous_after.min(mono.len().saturating_sub(written));
        for n in 0..right {
            std::mem::swap(&mut output[written + n], &mut seam[radius + n]);
        }
        let count = left + right;
        if count > 1 {
            for n in 0..count {
                let a = f64::from(seam[radius - left + n]);
                let b = f64::from(output[written - left + n]);
                let denominator = (count - 1) as f64;
                output[written - left + n] = bounded(
                    ((a / denominator) * n as f64 + (b / denominator) * (count - 1 - n) as f64)
                        as i32,
                );
            }
        }
        previous_after = radius.min(mono.len() - source - copy);
        seam[radius..radius + previous_after]
            .copy_from_slice(&mono[source + copy..source + copy + previous_after]);
        written += copy;
        source += copy + skip;
        ensure!(source > prior_source, "voice compression did not advance");
    }
    if written >= 100 {
        for n in 0..100 {
            let at = written - 100 + n;
            output[at] = (i32::from(output[at]) * (100 - n) as i32 / 100) as i16;
        }
    }
    output.truncate(written);
    Ok(Pcm16 {
        rate,
        channels: 1,
        samples: output,
    })
}
