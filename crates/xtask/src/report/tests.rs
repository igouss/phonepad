use super::{Report, line};

#[test]
fn a_passing_check_reads_pass() {
    assert_eq!(line("gate", true, "fine"), "PASS gate: fine");
}

#[test]
fn a_failing_check_reads_fail() {
    assert_eq!(line("gate", false, "broken"), "FAIL gate: broken");
}

#[test]
fn a_report_with_no_lines_passes() {
    assert!(Report::default().passed());
}

#[test]
fn a_report_with_one_passing_line_passes() {
    let mut report: Report = Report::default();
    report.line("gate", true, String::from("fine"));
    assert!(report.passed());
}

#[test]
fn a_failure_before_a_pass_fails_the_report() {
    let mut report: Report = Report::default();
    report.line("first", false, String::from("broken"));
    report.line("second", true, String::from("fine"));
    assert!(!report.passed());
}

#[test]
fn a_recorded_failing_check_fails_the_report() {
    let mut report: Report = Report::default();
    report.record(super::Check {
        gate: String::from("gate"),
        ok: false,
        detail: String::from("broken"),
    });
    assert!(!report.passed());
}
