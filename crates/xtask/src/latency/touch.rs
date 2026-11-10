use super::browser::Browser;
use super::stats::{self, Spread};
use crate::report::Check;
use serde_json::Value;
use std::f64::consts::TAU;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const PROBE: &str = "globalThis.padProbe = { draws: [], jumps: [], drawn(event, handled, at) { this.draws.push([event, handled, at]); }, reconciled(before, after, heading) { if (before && after) this.jumps.push([after.x - before.x, after.y - before.y, heading.x, heading.y]); }, take() { const taken = JSON.stringify({ draws: this.draws, jumps: this.jumps }); this.draws = []; this.jumps = []; return taken; } };";
const TAKE: &str = "padProbe.take()";
const PAD_CENTRE: &str = "JSON.stringify((r => [r.left + r.width / 2, r.top + r.height / 2])(pad.getBoundingClientRect()))";
pub const STEPS_PER_TURN: u32 = 120;
pub const RADIUS_CSS_PX: f64 = 60.0;
pub const REDRAW_P95_MS: f64 = 20.0;
pub const BACKWARD_MAX_PX: f64 = 4.0;

#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    pub draws: Vec<[f64; 3]>,
    pub jumps: Vec<[f64; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Phase {
    pub redraw: Option<Spread>,
    pub queued_redraw: Option<Spread>,
    pub reconciled: usize,
    pub backward_max: Option<f64>,
    pub jump_max: Option<f64>,
}

pub struct Driver {
    thread: JoinHandle<Result<Vec<Phase>, String>>,
}

pub fn backward(jump: [f64; 4]) -> f64 {
    let [jx, jy, hx, hy]: [f64; 4] = jump;
    let heading: f64 = hx.hypot(hy);
    if heading == 0.0 {
        jx.hypot(jy)
    } else {
        (-(jx * hx + jy * hy) / heading).max(0.0)
    }
}

fn largest(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.max_by(f64::total_cmp)
}

pub fn phase(taken: &Taken) -> Phase {
    Phase {
        redraw: stats::spread(
            taken
                .draws
                .iter()
                .map(|[_, handled, at]| at - handled)
                .collect(),
        ),
        queued_redraw: stats::spread(
            taken
                .draws
                .iter()
                .map(|[event, _, at]| at - event)
                .collect(),
        ),
        reconciled: taken.jumps.len(),
        backward_max: largest(taken.jumps.iter().copied().map(backward)),
        jump_max: largest(taken.jumps.iter().map(|[jx, jy, _, _]| jx.hypot(*jy))),
    }
}

pub fn finger(centre: (f64, f64), step: u32) -> (f64, f64) {
    let angle: f64 = f64::from(step % STEPS_PER_TURN) * TAU / f64::from(STEPS_PER_TURN);
    (
        centre.0 + RADIUS_CSS_PX * angle.cos(),
        centre.1 + RADIUS_CSS_PX * angle.sin(),
    )
}

fn rows<const N: usize>(value: &Value) -> Option<Vec<[f64; N]>> {
    value
        .as_array()?
        .iter()
        .map(|row: &Value| {
            let numbers: Vec<f64> = row
                .as_array()?
                .iter()
                .map(Value::as_f64)
                .collect::<Option<Vec<f64>>>()?;
            numbers.try_into().ok()
        })
        .collect()
}

pub fn parse(text: &str) -> Result<Taken, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("probe: {e}"))?;
    Ok(Taken {
        draws: rows(&value["draws"]).ok_or_else(|| format!("probe draws: {text}"))?,
        jumps: rows(&value["jumps"]).ok_or_else(|| format!("probe jumps: {text}"))?,
    })
}

fn take(browser: &mut Browser) -> Result<Taken, String> {
    parse(&browser.evaluate(TAKE)?)
}

fn circle(
    browser: &mut Browser,
    centre: (f64, f64),
    step: &mut u32,
    until: Instant,
) -> Result<(), String> {
    while Instant::now() < until {
        *step += 1;
        browser.touch("touchMove", Some(finger(centre, *step)))?;
    }
    Ok(())
}

pub fn window(index: u32, settle: Duration, measured: Duration) -> (Duration, Duration) {
    let settled: Duration = (settle + measured) * index + settle;
    (settled, settled + measured)
}

fn drive(
    browser: &mut Browser,
    phases: usize,
    settle: Duration,
    measured: Duration,
) -> Result<Vec<Phase>, String> {
    let centre: (f64, f64) = serde_json::from_str(&browser.evaluate(PAD_CENTRE)?)
        .map_err(|e| format!("pad centre: {e}"))?;
    let start: Instant = Instant::now();
    let mut step: u32 = 0;
    (0..u32::try_from(phases).map_err(|e| e.to_string())?)
        .map(|index: u32| {
            let (settled, ended): (Duration, Duration) = window(index, settle, measured);
            browser.touch("touchStart", Some(finger(centre, step)))?;
            circle(browser, centre, &mut step, start + settled)?;
            take(browser)?;
            circle(browser, centre, &mut step, start + ended)?;
            let taken: Taken = take(browser)?;
            browser.touch("touchEnd", None)?;
            Ok(phase(&taken))
        })
        .collect()
}

impl Driver {
    pub fn start(
        mut browser: Browser,
        phases: usize,
        settle: Duration,
        measured: Duration,
    ) -> Driver {
        Driver {
            thread: thread::spawn(move || {
                let driven: Result<Vec<Phase>, String> =
                    drive(&mut browser, phases, settle, measured);
                browser.close();
                driven
            }),
        }
    }

    pub fn finish(self) -> Result<Vec<Phase>, String> {
        self.thread
            .join()
            .map_err(|_| String::from("the touch driver panicked"))?
    }
}

fn ms(spread: Option<Spread>) -> String {
    spread.map_or_else(
        || String::from("none"),
        |s: Spread| format!("p50 {:.1} / p95 {:.1} / max {:.1} ms", s.p50, s.p95, s.max),
    )
}

fn px(value: Option<f64>) -> String {
    value.map_or_else(|| String::from("none"), |v: f64| format!("{v:.1} px"))
}

pub fn checks(profile: &str, phase: &Phase) -> [Check; 2] {
    [
        Check {
            gate: format!("{profile} touch→ring-redraw"),
            ok: phase.redraw.is_some_and(|s: Spread| s.p95 <= REDRAW_P95_MS),
            detail: format!(
                "handler→redraw {} (≤ {REDRAW_P95_MS} ms at p95); event timestamp→redraw {}",
                ms(phase.redraw),
                ms(phase.queued_redraw)
            ),
        },
        Check {
            gate: format!("{profile} reconciliation"),
            ok: phase
                .backward_max
                .is_some_and(|b: f64| b <= BACKWARD_MAX_PX),
            detail: format!(
                "{} pointer messages while touching, largest backward jump {} (≤ {BACKWARD_MAX_PX} px), largest jump {}",
                phase.reconciled,
                px(phase.backward_max),
                px(phase.jump_max)
            ),
        },
    ]
}

pub fn verdicts(profiles: &[&str], phases: &[Phase]) -> Vec<Check> {
    let counted: Check = Check {
        gate: String::from("touch phases"),
        ok: profiles.len() == phases.len(),
        detail: format!("{} of {} profiles measured", phases.len(), profiles.len()),
    };
    std::iter::once(counted)
        .chain(
            profiles
                .iter()
                .zip(phases)
                .flat_map(|(profile, phase): (&&str, &Phase)| checks(profile, phase)),
        )
        .collect()
}

#[cfg(test)]
mod tests;
