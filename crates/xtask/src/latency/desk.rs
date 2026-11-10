use super::content::{Content, Painted};
use super::nested::{Nested, Output, Placed};
use super::seen::{self, Rates, Stream};
use super::{Measured, Phase};
use crate::report::Report;
use crate::testbed::{Compositor, Scratch};
use crate::viewer::Received;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const PLACED_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);

pub struct Desk {
    nested: Nested,
    pub content: Content,
    pub output: Output,
}

fn placed(nested: &Nested) -> Result<Vec<Placed>, String> {
    let start: Instant = Instant::now();
    loop {
        let clients: Vec<Placed> = nested.clients()?;
        if !clients.is_empty() || start.elapsed() > PLACED_TIMEOUT {
            return Ok(clients);
        }
        thread::sleep(POLL);
    }
}

fn per_second(count: u64, over: Duration) -> f64 {
    count as f64 / over.as_secs_f64()
}

impl Desk {
    pub fn start(report: &mut Report, hyprland: &Path, scratch: &Scratch) -> Result<Desk, String> {
        let nested: Nested = Nested::start(hyprland, &scratch.dir)?;
        let output: Output = nested.output()?;
        let content: Content =
            Content::start(&nested.wayland_socket()?, (output.width, output.height), 0)?;
        let full: Placed = Placed {
            at: (0, 0),
            size: (i64::from(output.width), i64::from(output.height)),
        };
        let clients: Vec<Placed> = placed(&nested)?;
        report.line(
            "content-window",
            clients == [full] && content.size == (output.width, output.height),
            format!(
                "nested {} ({}x{} at {:.2} Hz, display {}): content buffer {}x{}, clients {clients:?}",
                hyprland.display(),
                output.width,
                output.height,
                output.refresh_hz,
                nested.compositor.display,
                content.size.0,
                content.size.1
            ),
        );
        nested.park((output.width / 2, output.height / 2))?;
        Ok(Desk {
            nested,
            content,
            output,
        })
    }

    pub fn compositor(&self) -> Compositor {
        self.nested.compositor.clone()
    }

    pub fn rates(
        &self,
        scratch: &Scratch,
        received: &Received,
        (phases, measured): (&[Phase], &[Measured]),
        over: Duration,
    ) -> Result<Vec<Rates>, String> {
        let stream: Stream = seen::stream(&received.binary);
        let h264: PathBuf = scratch.dir.join("viewer.h264");
        std::fs::write(&h264, &stream.h264).map_err(|e| format!("{}: {e}", h264.display()))?;
        let at: (u32, u32) = seen::sample_at(
            &stream.shown,
            seen::target((self.output.width, self.output.height)),
        )?;
        let changed: BTreeSet<u32> = seen::changed(&stream.shown, &seen::decode(&h264, at)?)?;
        println!(
            "INFO viewer saw {} frames, sampled at {at:?} in the encoded window",
            stream.shown.len()
        );
        Ok(phases
            .iter()
            .zip(measured)
            .filter_map(|(phase, measured): (&Phase, &Measured)| {
                let (before, after): (Painted, Painted) = measured.painted?;
                Some(Rates {
                    hz: phase.hz?,
                    painted_per_s: per_second(after.commits - before.commits, over),
                    skipped_per_s: per_second(after.skipped - before.skipped, over),
                    changes_per_s: seen::per_second(&changed, &measured.acked, over),
                    units_per_s: measured.summary.units_per_s,
                })
            })
            .collect())
    }

    pub fn stop(self, report: &mut Report) -> Result<(), String> {
        self.content.stop()?;
        let code: Option<i32> = self.nested.stop()?;
        report.line(
            "nested-stopped",
            true,
            format!("nested Hyprland exited with {code:?}"),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
