//! Where energy samples come from.
//!
//! v1 has one implementation, [`EnergiBridgeProcess`], which spawns the
//! `energibridge` binary and parses its CSV. The trait is the seam for a v2
//! `EnergiBridgeLib` that links EnergiBridge's sensor code directly, reads
//! only the energy counters and can change its sampling rate on the fly.

mod energibridge_process;

pub use energibridge_process::EnergiBridgeProcess;

use crate::integrate::Sample;
use std::io;

pub trait Sensor {
    /// Blocks until the next sample. `Ok(None)` means the sensor has ended.
    fn next_sample(&mut self) -> io::Result<Option<Sample>>;

    /// Short name without spaces, e.g. `smc-system` or `rapl-package+dram`.
    fn source(&self) -> &str;
}
