use crate::{guards::patch::safe_path, state::read_json};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, fs, path::Path};

pub fn approved(execution: &Value, artifacts: &Value) -> bool {
    let assessment = &artifacts["test-validator"]["coverage_assessment"];
    execution["coverage"]["status"] == "passed"
        && assessment["scope_valid"] == true
        && assessment["measurement_valid"] == true
        && assessment["meaningful_assertions"] == true
}

pub fn read_coverage(workspace: &Path, directory: &Path, specification: &Value) -> Result<Value> {
    let filename = specification["report"]
        .as_str()
        .context("Missing coverage report")?;
    let report = safe_path(directory, filename)?;
    let document = read_json(&report)?;
    let expected: HashSet<&str> = specification["files"]
        .as_array()
        .context("Missing coverage scope")?
        .iter()
        .map(|path| path.as_str().context("Invalid coverage scope"))
        .collect::<Result<_>>()?;
    ensure!(!expected.is_empty(), "Coverage scope is empty");
    let raw = document["raw_reports"]
        .as_array()
        .context("Missing original coverage reports")?;
    ensure!(!raw.is_empty(), "Missing original coverage reports");
    for path in raw {
        let path = safe_path(
            directory,
            path.as_str().context("Invalid coverage evidence path")?,
        )?;
        ensure!(
            path != report && path.is_file() && path.metadata()?.len() > 0,
            "Original coverage report is unavailable"
        );
    }
    let files = document["files"]
        .as_array()
        .context("Missing per-file coverage")?;
    let mut seen = HashSet::new();
    let mut covered = 0_u64;
    let mut executable = 0_u64;
    for file in files {
        let relative = file["path"]
            .as_str()
            .context("Missing coverage source path")?;
        ensure!(
            expected.contains(relative) && seen.insert(relative),
            "Unexpected or duplicate coverage file: {relative}"
        );
        let source = safe_path(workspace, relative)?;
        let source_lines = fs::read(source)?
            .split_inclusive(|byte| *byte == b'\n')
            .count() as u64;
        let covered_lines = file["covered_lines"]
            .as_u64()
            .context("Invalid covered line count")?;
        let executable_lines = file["executable_lines"]
            .as_u64()
            .context("Invalid executable line count")?;
        ensure!(
            covered_lines <= executable_lines && executable_lines <= source_lines,
            "Impossible coverage counts for {relative}"
        );
        covered = covered
            .checked_add(covered_lines)
            .context("Coverage count overflow")?;
        executable = executable
            .checked_add(executable_lines)
            .context("Coverage count overflow")?;
    }
    ensure!(
        seen == expected,
        "Coverage report omits scoped production files"
    );
    ensure!(
        executable > 0,
        "Coverage has no executable production lines"
    );
    let passed = u128::from(covered) * 100 >= u128::from(executable) * 80;
    Ok(
        json!({"status":if passed {"passed"} else {"below_threshold"},"threshold":80,
        "percent":covered as f64 * 100.0 / executable as f64,"covered_lines":covered,
        "executable_lines":executable,"files":files,"raw_reports":raw,"report":report}),
    )
}
