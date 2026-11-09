use std::process::Output;
use std::time::Duration;

use tokio::process::Command;
use tokio::time;

const TOOL_BOUND: Duration = Duration::from_secs(2);

pub async fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    run_within(TOOL_BOUND, program, args).await
}

async fn run_within(bound: Duration, program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output: Output = time::timeout(
        bound,
        Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| format!("{program} {args:?}: no answer within {bound:?}; killed"))?
    .map_err(|err| format!("{program} {args:?}: {err}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "{program} {args:?}: {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[tokio::test]
    async fn a_tool_that_answers_returns_its_output() {
        assert_eq!(run("printf", &["ok"]).await, Ok(b"ok".to_vec()));
    }

    #[tokio::test]
    async fn a_failing_tool_is_an_error() {
        assert!(run("false", &[]).await.is_err());
    }

    #[tokio::test]
    async fn a_hung_tool_is_cut_off_at_the_bound() {
        let started: Instant = Instant::now();
        let outcome: Result<Vec<u8>, String> =
            run_within(Duration::from_millis(100), "sleep", &["30"]).await;
        assert_eq!(
            (outcome.is_err(), started.elapsed() < Duration::from_secs(5)),
            (true, true)
        );
    }
}
