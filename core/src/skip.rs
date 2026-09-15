//! Opening and ending detection from the audio itself.
//!
//! AniSkip is community submissions, so a show in its first weeks has none
//! (MAL 47158 answered 404 for every episode, 2026-09-11) and neither does
//! anything obscure. An opening, though, is the same recording in every
//! episode of a season. Fingerprint the head of two episodes, find the long
//! stretch they share, and that stretch is the opening; keep its fingerprint
//! and every later episode only has to be searched for it.
//!
//! The fingerprint is the Haitsma-Kalker (Philips) one: per frame, 33
//! log-spaced bands between 300 and 2000 Hz, one bit per adjacent band pair
//! saying whether the energy difference grew or shrank since the previous
//! frame. It survives re-encodes, loudness changes and the small timing
//! drift between two releases, which is all two episodes of one show differ
//! by in the part that matters. The input is mono 16-bit PCM at
//! `SAMPLE_RATE`, which the player extracts with a headless mpv.

/// What the PCM handed in is sampled at. Everything above 2000 Hz is thrown
/// away by the bands, so a higher rate would only cost decode time.
pub const SAMPLE_RATE: usize = 11_025;
const FRAME: usize = 2048;
/// 256 samples at 11025 Hz: a fingerprint every 23 ms, frames overlapping
/// by 7/8. The overlap is what the match rests on. At a 1024 hop two
/// episodes whose opening starts half a hop apart were fingerprinted on
/// grids 46 ms out of step, their prints disagreed on most bits, and a
/// 50-second opening shared across a 15-second offset (165375 samples,
/// 161.5 hops) was not found at all.
const HOP: usize = 256;
const BANDS: usize = 33;

/// Seconds per fingerprint.
pub fn frame_seconds() -> f64 {
    HOP as f64 / SAMPLE_RATE as f64
}

/// A frame this quiet carries no bits worth comparing: silence, and the
/// black between a cold open and the opening, is identical in every episode
/// and would otherwise match itself for as long as it lasts. Its print is
/// replaced by a stand-in that no other recording produces, so quiet never
/// matches quiet.
const QUIET_RMS: f64 = 150.0;

pub fn fingerprint(pcm: &[i16]) -> Vec<u32> {
    if pcm.len() < FRAME {
        return Vec::new();
    }
    let window: Vec<f64> = (0..FRAME)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (FRAME - 1) as f64).cos())
        .collect();
    let edges = band_edges();
    let frames = (pcm.len() - FRAME) / HOP + 1;
    let mut prints = Vec::with_capacity(frames);
    let mut previous: Option<[f64; BANDS + 1]> = None;
    let mut re = vec![0.0; FRAME];
    let mut im = vec![0.0; FRAME];
    // Mixed into every quiet frame's stand-in print. Seeded by position
    // alone, silence at the same second of two episodes got the same
    // stand-ins and matched after all; the salt is the audio itself, so two
    // different recordings never share one.
    let salt = pcm.iter().step_by(97).fold(0xcbf2_9ce4_8422_2325u64, |h, &s| {
        (h ^ s as u16 as u64).wrapping_mul(0x0100_0000_01b3)
    });
    for n in 0..frames {
        let chunk = &pcm[n * HOP..n * HOP + FRAME];
        let rms = (chunk.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / FRAME as f64).sqrt();
        for i in 0..FRAME {
            re[i] = chunk[i] as f64 * window[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        let mut energy = [0.0f64; BANDS + 1];
        for (b, e) in energy.iter_mut().enumerate() {
            let (lo, hi) = (edges[b], edges[b + 1].max(edges[b] + 1));
            *e = (lo..hi).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
        }
        let print = match previous {
            Some(prev) if rms >= QUIET_RMS => {
                let mut bits = 0u32;
                for m in 0..BANDS - 1 {
                    let now = energy[m] - energy[m + 1];
                    let before = prev[m] - prev[m + 1];
                    if now - before > 0.0 {
                        bits |= 1 << m;
                    }
                }
                bits
            }
            _ => splitmix(n as u64 ^ salt) as u32,
        };
        prints.push(print);
        previous = Some(energy);
    }
    prints
}

/// FFT bin indices bounding the bands, log-spaced from 300 to 2000 Hz.
fn band_edges() -> [usize; BANDS + 2] {
    let mut edges = [0usize; BANDS + 2];
    let (lo, hi) = (300.0f64, 2000.0f64);
    for (i, edge) in edges.iter_mut().enumerate() {
        let f = lo * (hi / lo).powf(i as f64 / (BANDS + 1) as f64);
        *edge = (f * FRAME as f64 / SAMPLE_RATE as f64).round() as usize;
    }
    edges
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// In-place iterative radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

/// Frames are compared in blocks of this many (~3 s), never one at a time.
/// Two real episodes' copies of one ending (Tomodachi no Imouto 04 and 05,
/// two encodes of the same Crunchyroll master) disagreed on 0.21 to 0.41 of
/// their bits per 3 s block at the right alignment; a per-frame rule of at
/// most 10 differing bits caught only half the frames there and found 14 s
/// of an 89 s ending.
const BLOCK: usize = 128;
/// The block bit error rate a shared stretch stays under. On that same pair
/// the ending's blocks peaked at 0.41 and everything outside it, and every
/// block at a wrong alignment, sat at 0.48 or above; unrelated prints
/// disagree on half their bits.
const MAX_BLOCK_BER: f64 = 0.43;

/// A stretch two fingerprints share: `len` frames from `a_start` in the
/// first and `b_start` in the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedRun {
    pub a_start: usize,
    pub b_start: usize,
    pub len: usize,
}

/// The longest stretch of `a` that also occurs in `b` at one alignment.
// Index loops on purpose: `i` and `j` walk two slices at a fixed offset, and
// the iterator form of that is two zips and a skip that hide the alignment.
#[allow(clippy::needless_range_loop)]
pub fn longest_shared_run(a: &[u32], b: &[u32]) -> Option<SharedRun> {
    if a.len() < BLOCK || b.len() < BLOCK {
        return None;
    }
    let limit = (MAX_BLOCK_BER * (BLOCK * 32) as f64) as u32;
    let mut best: Option<SharedRun> = None;
    let mut errors: Vec<u32> = Vec::with_capacity(a.len().min(b.len()));
    // Offset d aligns a[i] with b[i - d].
    for d in -(b.len() as isize - 1)..a.len() as isize {
        let i0 = d.max(0) as usize;
        let i1 = (b.len() as isize + d).min(a.len() as isize) as usize;
        if i1 - i0 < BLOCK || i1 - i0 <= best.map_or(0, |r| r.len) {
            continue;
        }
        errors.clear();
        for i in i0..i1 {
            errors.push((a[i] ^ b[(i as isize - d) as usize]).count_ones());
        }
        // A block that clears the bar covers its centre; the run is from the
        // first such centre to the last. Taking the blocks' outer edges
        // instead made every span start and end up to 3 s into the scenes
        // around it, since a block half inside an opening can already pass.
        let mut sum: u32 = errors[..BLOCK].iter().sum();
        let mut run_start: Option<usize> = None;
        for k in 0..=errors.len() - BLOCK {
            if k > 0 {
                sum = sum - errors[k - 1] + errors[k + BLOCK - 1];
            }
            let good = sum <= limit;
            match (good, run_start) {
                (true, None) => run_start = Some(k),
                (false, Some(start)) => {
                    let len = k - start;
                    if len > best.map_or(0, |r| r.len) {
                        best = Some(SharedRun { a_start: i0 + start + BLOCK / 2, b_start: (i0 as isize + (start + BLOCK / 2) as isize - d) as usize, len });
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run_start {
            let len = errors.len() - BLOCK + 1 - start;
            if len > best.map_or(0, |r| r.len) {
                best = Some(SharedRun { a_start: i0 + start + BLOCK / 2, b_start: (i0 as isize + (start + BLOCK / 2) as isize - d) as usize, len });
            }
        }
    }
    best
}

/// Openings and endings run a minute and a half, give or take. Shorter than
/// this is a shared sting or a title card; longer is a recap or a whole
/// repeated scene, and skipping either would skip story.
pub const MIN_SEGMENT_SECONDS: f64 = 15.0;
pub const MAX_SEGMENT_SECONDS: f64 = 180.0;

/// A detected span, in seconds of the file the fingerprint was taken from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

/// The segment two episodes share, as a span of the first and the
/// fingerprint of it (the reference later episodes are searched for).
/// `offset_*` is where in each file its fingerprint begins.
pub fn detect_shared(a: &[u32], offset_a: f64, b: &[u32]) -> Option<(Span, Vec<u32>)> {
    let run = longest_shared_run(a, b)?;
    let seconds = run.len as f64 * frame_seconds();
    if !(MIN_SEGMENT_SECONDS..=MAX_SEGMENT_SECONDS).contains(&seconds) {
        return None;
    }
    let start = offset_a + run.a_start as f64 * frame_seconds();
    Some((
        Span { start, end: start + seconds },
        a[run.a_start..run.a_start + run.len].to_vec(),
    ))
}

/// A stored reference found in one episode: where it sits there. Most of the
/// reference has to be present, and the span is the reference's full extent
/// at that alignment, so an opening whose first seconds are under a sound
/// effect still skips from its true start.
pub fn find_reference(reference: &[u32], target: &[u32], offset: f64) -> Option<Span> {
    let run = longest_shared_run(reference, target)?;
    if (run.len as f64) < reference.len() as f64 * 0.6 {
        return None;
    }
    let start_frame = run.b_start as isize - run.a_start as isize;
    let start = offset + start_frame.max(0) as f64 * frame_seconds();
    let end = offset + (start_frame + reference.len() as isize).max(0) as f64 * frame_seconds();
    (end - start >= MIN_SEGMENT_SECONDS).then_some(Span { start, end })
}

/// Reads raw mono s16le PCM as mpv's `ao=pcm` writes it with no header.
pub fn read_pcm(path: &std::path::Path) -> Result<Vec<i16>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())
}

pub fn prints_to_bytes(prints: &[u32]) -> Vec<u8> {
    prints.iter().flat_map(|p| p.to_le_bytes()).collect()
}

pub fn prints_from_bytes(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise with a slowly moving pitch, so the bands change
    /// from frame to frame the way music does. Different seeds are
    /// different "scenes".
    fn scene(seed: u64, seconds: f64) -> Vec<i16> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        let mut state = seed;
        (0..n)
            .map(|i| {
                state = splitmix(state);
                let noise = (state % 2000) as f64 - 1000.0;
                let t = i as f64 / SAMPLE_RATE as f64;
                let pitch = 400.0 + 800.0 * ((t * 0.7 + seed as f64).sin() * 0.5 + 0.5);
                let tone = (2.0 * std::f64::consts::PI * pitch * t).sin() * 6000.0;
                (tone + noise * 3.0).clamp(-32000.0, 32000.0) as i16
            })
            .collect()
    }

    fn cat(parts: &[&[i16]]) -> Vec<i16> {
        parts.iter().flat_map(|p| p.iter().copied()).collect()
    }

    #[test]
    fn two_episodes_share_their_opening() {
        let opening = scene(7, 60.0);
        let ep1 = cat(&[&scene(1, 40.0), &opening, &scene(2, 120.0)]);
        // A longer cold open, and the opening a little quieter.
        let quieter: Vec<i16> = opening.iter().map(|&s| (s as f64 * 0.7) as i16).collect();
        let ep2 = cat(&[&scene(3, 95.0), &quieter, &scene(4, 90.0)]);
        let (span, reference) = detect_shared(&fingerprint(&ep1), 0.0, &fingerprint(&ep2)).expect("opening found");
        assert!((span.start - 40.0).abs() < 1.5, "start {}", span.start);
        assert!((span.end - 100.0).abs() < 1.5, "end {}", span.end);

        let ep3 = cat(&[&scene(5, 10.0), &opening, &scene(6, 60.0)]);
        let found = find_reference(&reference, &fingerprint(&ep3), 0.0).expect("reference found");
        assert!((found.start - 10.0).abs() < 1.5, "start {}", found.start);
        assert!((found.end - 70.0).abs() < 1.5, "end {}", found.end);
    }

    #[test]
    fn unrelated_episodes_share_nothing() {
        let ep1 = cat(&[&scene(11, 90.0), &scene(12, 90.0)]);
        let ep2 = cat(&[&scene(13, 90.0), &scene(14, 90.0)]);
        assert!(detect_shared(&fingerprint(&ep1), 0.0, &fingerprint(&ep2)).is_none());
    }

    #[test]
    fn shared_silence_is_not_an_opening() {
        let silence = vec![0i16; 60 * SAMPLE_RATE];
        let ep1 = cat(&[&scene(21, 30.0), &silence, &scene(22, 30.0)]);
        let ep2 = cat(&[&scene(23, 30.0), &silence, &scene(24, 30.0)]);
        assert!(detect_shared(&fingerprint(&ep1), 0.0, &fingerprint(&ep2)).is_none());
    }

    #[test]
    fn an_offset_moves_the_span() {
        let opening = scene(31, 50.0);
        let ep1 = cat(&[&scene(32, 20.0), &opening, &scene(33, 20.0)]);
        let ep2 = cat(&[&scene(34, 5.0), &opening, &scene(35, 20.0)]);
        let (span, _) = detect_shared(&fingerprint(&ep1), 1200.0, &fingerprint(&ep2)).unwrap();
        assert!((span.start - 1220.0).abs() < 1.5);
    }

    /// Real audio from local files, which a checkout does not have:
    /// `SKIP_A`/`SKIP_B` are raw mono s16le at 11025 Hz (see `read_pcm`).
    /// Prints the longest shared run and how long matching took.
    #[test]
    #[ignore]
    fn real_audio_pair() {
        let (Ok(a), Ok(b)) = (std::env::var("SKIP_A"), std::env::var("SKIP_B")) else { return };
        let a = fingerprint(&read_pcm(std::path::Path::new(&a)).unwrap());
        let b = fingerprint(&read_pcm(std::path::Path::new(&b)).unwrap());
        let started = std::time::Instant::now();
        let run = longest_shared_run(&a, &b);
        eprintln!(
            "frames {} x {}, {:?} in {:?}, run {:?}",
            a.len(),
            b.len(),
            run.map(|r| (r.a_start as f64 * frame_seconds(), r.b_start as f64 * frame_seconds(), r.len as f64 * frame_seconds())),
            started.elapsed(),
            run
        );
    }

    #[test]
    fn prints_round_trip_through_bytes() {
        let prints = vec![0, 1, u32::MAX, 0xdead_beef];
        assert_eq!(prints_from_bytes(&prints_to_bytes(&prints)), prints);
    }
}
