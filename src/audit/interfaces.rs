use crate::{agents::codex::Codex, runtime::compose::ComposeEnvironment};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub trait AuditAgent {
    fn coordinate(&self, harness: &mut crate::audit::harness::AuditHarness<'_>) -> Result<()>;
    fn invoke(
        &self,
        stage: &str,
        workspace: &Path,
        directory: &Path,
        inputs: &Value,
        progress: &dyn Fn(&str),
    ) -> Result<Value>;
}
impl AuditAgent for Codex {
    fn coordinate(&self, harness: &mut crate::audit::harness::AuditHarness<'_>) -> Result<()> {
        Codex::coordinate(self, harness)
    }
    fn invoke(
        &self,
        stage: &str,
        workspace: &Path,
        directory: &Path,
        inputs: &Value,
        progress: &dyn Fn(&str),
    ) -> Result<Value> {
        Codex::invoke(self, stage, workspace, directory, inputs, progress)
    }
}
pub trait TestRuntime {
    fn start(&self) -> Result<()>;
    fn inspect(&self, _probe: &Value, _timeout: u64) -> Result<Value> {
        anyhow::bail!("This runtime does not support runner probes")
    }
    fn execute_test(&self, command: &[String], test_id: &str, timeout: u64) -> Result<Value>;
    fn logs(&self) -> Result<()>;
}
impl TestRuntime for ComposeEnvironment {
    fn start(&self) -> Result<()> {
        ComposeEnvironment::start(self)
    }
    fn inspect(&self, probe: &Value, timeout: u64) -> Result<Value> {
        ComposeEnvironment::inspect(self, probe, timeout)
    }
    fn execute_test(&self, command: &[String], test_id: &str, timeout: u64) -> Result<Value> {
        ComposeEnvironment::execute_test(self, command, test_id, timeout)
    }
    fn logs(&self) -> Result<()> {
        ComposeEnvironment::logs(self)
    }
}
pub type RuntimeFactory<'a> = dyn Fn(&Path, &Path, &str, Value, u64) -> Box<dyn TestRuntime> + 'a;
