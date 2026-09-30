use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};

use super::Sensor;
use crate::csv::Columns;
use crate::integrate::Sample;

/// `energibridge -i <interval> -- <watchdog command>` with its CSV on a pipe.
///
/// EnergiBridge samples until the command it measures exits, so the watchdog
/// decides how long the sensor lives.
pub struct EnergiBridgeProcess {
    child: Child,
    lines: BufReader<ChildStdout>,
    columns: Columns,
    line: String,
}

impl EnergiBridgeProcess {
    pub fn spawn(
        energibridge: &Path,
        interval_ms: u64,
        watchdog: &[String],
        stderr: File,
    ) -> io::Result<Self> {
        let mut child = Command::new(energibridge)
            .arg("-i")
            .arg(interval_ms.to_string())
            .arg("--")
            .args(watchdog)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()?;
        let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut header = String::new();
        lines.read_line(&mut header)?;
        let Some(columns) = Columns::from_header(&header) else {
            let _ = child.kill();
            let _ = child.wait();
            let what = if header.is_empty() {
                "energibridge exited without output".to_string()
            } else {
                format!("no energy column in header: {}", header.trim_end())
            };
            return Err(io::Error::new(io::ErrorKind::InvalidData, what));
        };
        Ok(Self {
            child,
            lines,
            columns,
            line: String::new(),
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

impl Sensor for EnergiBridgeProcess {
    fn next_sample(&mut self) -> io::Result<Option<Sample>> {
        loop {
            self.line.clear();
            if self.lines.read_line(&mut self.line)? == 0 {
                return Ok(None);
            }
            // Rows that do not parse (a panic message, a torn line) are skipped.
            if let Some(sample) = self.columns.parse_row(&self.line) {
                return Ok(Some(sample));
            }
        }
    }

    fn source(&self) -> &str {
        self.columns.source
    }
}
