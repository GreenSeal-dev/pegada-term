//! The state file the shell hooks read.
//!
//! Line 1: `seq t_ms energy_mJ power_mW idle_mW interval_ms source res_ms seq`
//! Lines 2-4: power history in mW, newest first, at 1x, 10x and 100x the
//! sample interval, up to 64 entries each.
//! A last line of spaces pads the file to a multiple of [`PAD`] bytes.
//!
//! `res_ms` is how often the sensor value really changes. It can be coarser
//! than the sample interval (the Apple Silicon SMC updates about once a second).
//!
//! The file is rewritten in place with a single `pwrite`, not replaced with
//! temp file + rename: on macOS a create + rename per sample costs about
//! fifteen times more CPU (measured: ~1.6 ms vs ~0.1 ms), and an unchanged
//! inode lets the zsh hook keep the file open instead of reopening it on
//! every prompt. `seq` is repeated at the end of line 1 so a reader can
//! reject the (very unlikely) line caught in the middle of a write.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

pub const HISTORY_LEN: usize = 64;
const LEVELS: usize = 3;
const STEP: u32 = 10;
/// The file only changes length when the content crosses a multiple of this.
const PAD: usize = 256;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub seq: u64,
    pub t_ms: u64,
    pub energy_mj: u64,
    pub power_mw: u64,
    pub idle_mw: u64,
    pub interval_ms: u64,
    pub source: String,
    pub res_ms: u64,
    /// Newest first.
    pub history: [Vec<u32>; LEVELS],
}

impl State {
    pub fn encode(&self) -> String {
        let mut out = String::with_capacity(1024);
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} {} {} {}",
            self.seq,
            self.t_ms,
            self.energy_mj,
            self.power_mw,
            self.idle_mw,
            self.interval_ms,
            self.source,
            self.res_ms,
            self.seq
        );
        for level in &self.history {
            for (i, v) in level.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                let _ = write!(out, "{v}");
            }
            out.push('\n');
        }
        out
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let mut f = lines.next()?.split(' ');
        let mut num = || f.next()?.parse::<u64>().ok();
        let (seq, t_ms, energy_mj, power_mw, idle_mw, interval_ms) =
            (num()?, num()?, num()?, num()?, num()?, num()?);
        let source = f.next()?.to_string();
        let res_ms = f.next().and_then(|v| v.parse().ok()).unwrap_or(interval_ms);
        // The trailing copy of seq differs only if the line was read mid-write.
        if f.next().is_some_and(|guard| guard.parse() != Ok(seq)) {
            return None;
        }
        let mut history: [Vec<u32>; LEVELS] = Default::default();
        for level in &mut history {
            *level = lines
                .next()
                .unwrap_or("")
                .split(' ')
                .filter_map(|v| v.parse().ok())
                .collect();
        }
        Some(Self {
            seq,
            t_ms,
            energy_mj,
            power_mw,
            idle_mw,
            interval_ms,
            source,
            res_ms,
            history,
        })
    }

    pub fn read(path: &Path) -> Option<Self> {
        Self::parse(&fs::read_to_string(path).ok()?)
    }

    /// A state this old means the sampler is gone (same rule as the hooks).
    pub fn is_fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.t_ms) <= 4 * self.interval_ms + 2000
    }
}

/// Writes the state file in place, one `pwrite` per sample.
pub struct StateWriter {
    path: PathBuf,
    file: File,
    len: usize,
    buf: String,
}

impl StateWriter {
    /// Opens the existing file (shells may hold it open) or creates it.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            len: file.metadata()?.len() as usize,
            file,
            buf: String::with_capacity(8 * PAD),
        })
    }

    pub fn write(&mut self, state: &State) -> io::Result<()> {
        self.buf.clear();
        self.buf.push_str(&state.encode());
        let padded = (self.buf.len() / PAD + 1) * PAD;
        while self.buf.len() < padded - 1 {
            self.buf.push(' ');
        }
        self.buf.push('\n');
        self.file.write_all_at(self.buf.as_bytes(), 0)?;
        if self.len != padded {
            self.file.set_len(padded as u64)?;
            self.len = padded;
        }
        Ok(())
    }

    /// Someone deleted the file (a /tmp cleaner): write to a new one from now on.
    pub fn reopen_if_missing(&mut self) -> io::Result<()> {
        if !self.path.exists() {
            *self = Self::open(&self.path)?;
        }
        Ok(())
    }
}

/// Three rings: every sample, the mean of every 10, the mean of every 100.
#[derive(Default)]
pub struct History {
    rings: [VecDeque<u32>; LEVELS],
    sums: [u64; LEVELS],
    counts: [u32; LEVELS],
}

impl History {
    pub fn push(&mut self, power_mw: u32) {
        let mut value = power_mw;
        for level in 0..LEVELS {
            let ring = &mut self.rings[level];
            ring.push_front(value);
            ring.truncate(HISTORY_LEN);
            self.sums[level] += u64::from(value);
            self.counts[level] += 1;
            if self.counts[level] < STEP {
                break;
            }
            value = (self.sums[level] / u64::from(STEP)) as u32;
            self.sums[level] = 0;
            self.counts[level] = 0;
        }
    }

    pub fn snapshot(&self) -> [Vec<u32>; LEVELS] {
        // Ring 0 holds samples; rings 1 and 2 hold the 10x and 100x means.
        let mut out: [Vec<u32>; LEVELS] = Default::default();
        for (dst, src) in out.iter_mut().zip(&self.rings) {
            dst.extend(src.iter());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_one_format_is_stable() {
        let s = State {
            seq: 7,
            t_ms: 1_790_762_788_155,
            energy_mj: 123_456,
            power_mw: 11_500,
            idle_mw: 7_900,
            interval_ms: 500,
            source: "smc-system".into(),
            res_ms: 1000,
            history: [vec![11_500, 11_000], vec![10_000], vec![]],
        };
        assert_eq!(
            s.encode(),
            "7 1790762788155 123456 11500 7900 500 smc-system 1000 7\n11500 11000\n10000\n\n"
        );
    }

    #[test]
    fn encode_parse_round_trip() {
        let s = State {
            seq: 1,
            t_ms: 2,
            energy_mj: 3,
            power_mw: 4,
            idle_mw: 5,
            interval_ms: 200,
            source: "rapl-package+dram".into(),
            res_ms: 200,
            history: [vec![9, 8, 7], vec![], vec![1]],
        };
        assert_eq!(State::parse(&s.encode()), Some(s));
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(State::parse(""), None);
        assert_eq!(State::parse("1 2 3\n"), None);
        assert_eq!(State::parse("a b c d e f g h\n"), None);
    }

    #[test]
    fn freshness_scales_with_interval() {
        let s = State {
            t_ms: 10_000,
            interval_ms: 500,
            ..Default::default()
        };
        assert!(s.is_fresh(10_000 + 4_000));
        assert!(!s.is_fresh(10_000 + 4_001));
        assert!(s.is_fresh(9_000));
    }

    #[test]
    fn history_levels_downsample_by_ten() {
        let mut h = History::default();
        for i in 1..=250u32 {
            h.push(i);
        }
        let [l1, l2, l3] = h.snapshot();
        assert_eq!(l1.len(), HISTORY_LEN);
        assert_eq!(&l1[..3], &[250, 249, 248]);
        // Means of 241..=250, 231..=240, ...
        assert_eq!(&l2[..3], &[245, 235, 225]);
        assert_eq!(l2.len(), 25);
        // Means of the first and second hundred, from the level-2 means.
        assert_eq!(l3, vec![150, 50]);
    }

    #[test]
    fn a_line_torn_by_a_concurrent_write_is_rejected() {
        let whole = "8 1000 5 6 7 500 smc-system 1000 8\n";
        assert!(State::parse(whole).is_some());
        // seq 8 at the start, the tail of seq 7's line at the end.
        assert_eq!(State::parse("8 1000 5 6 7 500 smc-system 1000 7\n"), None);
        // Files from before the guard existed still parse.
        assert!(State::parse("8 1000 5 6 7 500 smc-system 1000\n").is_some());
    }

    #[test]
    fn writer_rewrites_in_place_and_pads() {
        let dir = std::env::temp_dir().join(format!("pegada-term-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state");
        let mut w = StateWriter::open(&path).unwrap();
        let mut s = State {
            seq: 1,
            interval_ms: 500,
            source: "smc-system".into(),
            res_ms: 500,
            history: [vec![123_456; 64], vec![], vec![]],
            ..Default::default()
        };
        w.write(&s).unwrap();
        let inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path).unwrap().ino()
        };
        let long = fs::read_to_string(&path).unwrap();
        assert_eq!(long.len() % PAD, 0);
        assert!(long.ends_with(" \n"));
        assert_eq!(State::read(&path).unwrap(), s);

        // A shorter state leaves nothing of the longer one behind.
        s.seq = 2;
        s.history = [vec![9], vec![], vec![]];
        w.write(&s).unwrap();
        let short = fs::read_to_string(&path).unwrap();
        assert!(short.len() < long.len() && short.len() % PAD == 0);
        assert_eq!(State::read(&path).unwrap(), s);
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&path).unwrap().ino(),
                inode,
                "the file must not be replaced"
            );
        }

        // A second writer (the next daemon) keeps the same file.
        let mut w2 = StateWriter::open(&path).unwrap();
        s.seq = 3;
        w2.write(&s).unwrap();
        assert_eq!(State::read(&path).unwrap(), s);

        fs::remove_file(&path).unwrap();
        w2.reopen_if_missing().unwrap();
        w2.write(&s).unwrap();
        assert_eq!(State::read(&path).unwrap(), s);
        fs::remove_dir_all(&dir).unwrap();
    }
}
