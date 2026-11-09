pub struct Report {
    passed: bool,
}

impl Default for Report {
    fn default() -> Report {
        Report { passed: true }
    }
}

impl Report {
    pub fn line(&mut self, gate: &str, ok: bool, detail: String) {
        println!("{}", line(gate, ok, &detail));
        self.passed &= ok;
    }

    pub fn passed(&self) -> bool {
        self.passed
    }
}

pub fn line(gate: &str, ok: bool, detail: &str) -> String {
    format!("{} {gate}: {detail}", if ok { "PASS" } else { "FAIL" })
}

#[cfg(test)]
mod tests;
