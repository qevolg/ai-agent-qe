//! Explicit recovery rounds retain historical attempts and invalidate active conclusions.
use super::*;

impl AuditHarness<'_> {
    pub fn next_stage(&self) -> &'static str {
        if self.artifacts.get("code-explorer").is_none() {
            return "code-explorer";
        }
        if self.artifacts.get("bug-hunter").is_none()
            || self.artifacts["bug-hunter"]["candidates"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            return "bug-hunter";
        }
        if self.artifacts.get("test-designer").is_none() {
            return "test-designer";
        }
        if !self.implementable() {
            return "test-validator";
        }
        if self.artifacts.get("test-developer").is_none() {
            return "test-developer";
        }
        if self.artifacts.get("test-validator").is_none() {
            return "test-validator";
        }
        if !crate::guards::coverage::approved(&self.execution, &self.artifacts) {
            "test-developer"
        } else {
            "test-validator"
        }
    }

    /// Caller owns RunLock. Returns the first stage of the new execution round.
    pub(crate) fn preflight_resume(&mut self, from_stage: Option<&str>) -> Result<(String, usize)> {
        ensure!(
            crate::runtime::policy::denial(&self.directory).is_none(),
            "Audit terminated after a safety policy denial"
        );
        ensure!(
            !["running", "initializing"]
                .iter()
                .any(|s| self.state["status"] == *s),
            "Run is already active; use the existing coordinator"
        );
        ensure!(
            self.state["environment_cleaned_at"].is_null(),
            "Run environment was cleaned; start a new audit"
        );
        let stage = from_stage.unwrap_or_else(|| self.next_stage()).to_owned();
        let index = STAGES
            .iter()
            .position(|s| *s == stage)
            .context("Unknown resume stage")?;
        for prerequisite in &STAGES[..if index == 4 { 3 } else { index }] {
            ensure!(
                self.artifacts.get(*prerequisite).is_some(),
                "Missing resume prerequisite: {prerequisite}"
            );
        }
        self.recover_baseline()?;
        self.recover_transition()?;
        self.gate()?.verify_worktree()?;
        Ok((stage, index))
    }

    pub fn resume(&mut self, from_stage: Option<&str>) -> Result<String> {
        let (stage, index) = self.preflight_resume(from_stage)?;
        let archives = self.directory.join("resumes");
        let previous_round = fs::read_dir(&archives)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let round = self.state["resume_round"]
            .as_u64()
            .unwrap_or(0)
            .max(previous_round)
            + 1;
        let archive = self.directory.join("resumes").join(format!("{round:04}"));
        fs::create_dir_all(archive.parent().unwrap())?;
        fs::create_dir(&archive)
            .context("Resume archive already exists; inspect before retrying")?;
        self.save()?;
        for name in [
            "harness.json",
            "run.json",
            "execution.json",
            "report.md",
            "report.json",
            "tests.diff",
            "patch-check.json",
        ] {
            let path = self.directory.join(name);
            if path.is_file() {
                fs::copy(path, archive.join(name))?;
            }
        }
        write_json(
            &self.directory.join("patch-transition.json"),
            &json!({"previous":self.state["applied_patch"],"replacement":if index < 3 { Value::Null } else { self.state["applied_patch"].clone() }}),
        )?;
        self.invalidate(index)?;
        // Retain Developer's cumulative patch as input while replacing its tests.
        if index != 3 {
            self.artifacts.as_object_mut().unwrap().remove(&stage);
            let alias = self
                .directory
                .join("artifacts")
                .join(format!("{stage}.json"));
            if alias.exists() {
                fs::remove_file(alias)?;
            }
        }
        if index == 4 {
            self.verdicts = json!([]);
        }
        for entry in self.state["stages"].as_array_mut().unwrap() {
            if entry["status"] == "running" {
                entry["status"] = json!("failed");
                entry["finished_at"] = json!(now());
                entry["error"] = json!("Previous worker exited before recording completion");
            }
        }
        let operations = self.state["environment_actions"]
            .as_array()
            .map_or(0, Vec::len)
            + self.state["execution_attempts"]
                .as_array()
                .map_or(0, Vec::len);
        self.state["round_offsets"] = json!({"stages":self.state["stages"].as_array().unwrap().len(),"operations":operations});
        self.state["resume_round"] = json!(round);
        self.state["resume_from_stage"] = json!(stage);
        self.state["status"] = json!("running");
        self.state["loop_started_at"] = json!(now());
        self.state["audit_finalized"] = json!(false);
        self.state["finished_at"] = Value::Null;
        self.state["error"] = Value::Null;
        self.state["finish_reason"] = Value::Null;
        self.state["recovery_error"] = Value::Null;
        self.state["resume_requires_execution"] =
            json!(index == 4 && self.implementable() && !self.execution_matches());
        for marker in ["audit-finished", "cancel"] {
            let path = self.directory.join("stages/coordinator").join(marker);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        self.save()?;
        self.recover_transition()?;
        Ok(stage)
    }

    fn execution_matches(&self) -> bool {
        let Some(tests) = self.artifacts["test-developer"]["tests"].as_array() else {
            return false;
        };
        let tests_match = tests.iter().all(|test| {
            let target = self.execution["tests"].as_array().is_some_and(|records| {
                records.iter().any(|record| {
                    record["test_id"] == test["id"]
                        && record["candidate_id"] == test["candidate_id"]
                        && record["evidence"]["complete"] == true
                        && persisted(record)
                })
            });
            let baselines: Vec<_> = self.execution["baseline"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|record| {
                    record["for_test_id"].is_null() || record["for_test_id"] == test["id"]
                })
                .collect();
            target
                && !baselines.is_empty()
                && baselines.iter().all(|record| {
                    record["result"]["returncode"] == 0
                        && record["result"]["timed_out"] != true
                        && persisted(record)
                })
        });
        let coverage = &self.execution["coverage"];
        let coverage_matches = persisted(&coverage["record"])
            && coverage["record"]["directory"]
                .as_str()
                .is_some_and(|path| {
                    self.root().is_ok_and(|root| {
                        crate::guards::coverage::read_coverage(
                            &root,
                            Path::new(path),
                            &self.artifacts["test-developer"]["coverage"],
                        )
                        .is_ok_and(|measurement| {
                            let mut expected = coverage.clone();
                            expected.as_object_mut().unwrap().remove("record");
                            measurement == expected
                        })
                    })
                });
        tests_match && coverage_matches
    }
}

fn persisted(record: &Value) -> bool {
    record["directory"].as_str().is_some_and(|path| {
        read_json(&Path::new(path).join("execution.json")).is_ok_and(|mut raw| {
            let mut expected = record.clone();
            // Older baseline records persisted before this harness annotation.
            for value in [&mut raw, &mut expected] {
                if let Some(object) = value.as_object_mut() {
                    object.remove("for_test_id");
                }
            }
            raw == expected
        })
    })
}
