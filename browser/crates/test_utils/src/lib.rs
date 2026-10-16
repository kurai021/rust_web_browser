//! Shared, data-only WPT accounting. Skips remain visible in archived reports.
use serde::{Deserialize, Serialize};
pub const WPT_HARNESS: &str = include_str!("wpt.js");
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Skip,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    pub status: Status,
    pub assertions: u64,
    #[serde(default)]
    pub detail: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct WptReport {
    pub suite: String,
    pub upstream: String,
    pub cases: Vec<Case>,
}
impl WptReport {
    pub fn summary(&self) -> (usize, usize, usize, u64) {
        let (mut pass, mut fail, mut skip, mut assertions) = (0, 0, 0, 0);
        for case in &self.cases {
            match case.status {
                Status::Pass => pass += 1,
                Status::Fail => fail += 1,
                Status::Skip => skip += 1,
            }
            assertions += case.assertions;
        }
        (pass, fail, skip, assertions)
    }
    pub fn publish(&self) -> std::io::Result<()> {
        let (pass, fail, skip, assertions) = self.summary();
        println!(
            "WPT {}: pass={pass} fail={fail} skip={skip} assertions={assertions}",
            self.suite
        );
        if let Some(directory) = std::env::var_os("BROWSER_WPT_REPORT_DIR") {
            let path = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&path)?;
            let name = self
                .suite
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            std::fs::write(
                path.join(format!("{name}.json")),
                serde_json::to_vec_pretty(self)?,
            )?;
        }
        Ok(())
    }
}
