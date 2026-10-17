//! Terminal process lifecycle; byte parsing, layout and rendering are host work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPhase {
    Launching,
    Running,
    Stopped,
    Failed,
}
#[derive(Clone, Debug)]
pub struct TerminalState {
    pub phase: TerminalPhase,
    pub process_id: Option<u32>,
    pub exit_code: Option<i32>,
    pub launch_error: Option<String>,
}
impl Default for TerminalState {
    fn default() -> Self {
        Self {
            phase: TerminalPhase::Launching,
            process_id: None,
            exit_code: None,
            launch_error: None,
        }
    }
}
impl TerminalState {
    pub fn input(
        &self,
        term_id: lapce_rpc::terminal::TermId,
        content: String,
    ) -> Result<Option<lapce_rpc::proxy::ProxyNotification>, String> {
        if matches!(self.phase, TerminalPhase::Stopped | TerminalPhase::Failed) {
            return Err("Terminal is not running".into());
        }
        if content.is_empty() {
            return Ok(None);
        }
        Ok(Some(lapce_rpc::proxy::ProxyNotification::TerminalWrite {
            term_id,
            content,
        }))
    }
    pub fn resize(
        &self,
        term_id: lapce_rpc::terminal::TermId,
        columns: usize,
        rows: usize,
    ) -> Result<lapce_rpc::proxy::ProxyNotification, String> {
        if matches!(self.phase, TerminalPhase::Stopped | TerminalPhase::Failed) {
            return Err("Terminal is not running".into());
        }
        let size = lapce_rpc::terminal::TerminalSize::clamp(columns, rows);
        Ok(lapce_rpc::proxy::ProxyNotification::TerminalResize {
            term_id,
            width: size.columns as usize,
            height: size.rows as usize,
        })
    }
    pub fn restart(&mut self) {
        *self = Self::default();
    }
    pub fn started(&mut self, process_id: Option<u32>) -> bool {
        if matches!(self.phase, TerminalPhase::Stopped | TerminalPhase::Failed) {
            return false;
        }
        self.phase = TerminalPhase::Running;
        self.process_id = process_id;
        true
    }
    pub fn stopped(&mut self, exit_code: Option<i32>) -> bool {
        if matches!(self.phase, TerminalPhase::Stopped | TerminalPhase::Failed) {
            return false;
        }
        self.phase = TerminalPhase::Stopped;
        self.exit_code = exit_code;
        true
    }
    pub fn failed(&mut self, error: String) -> bool {
        if matches!(self.phase, TerminalPhase::Stopped | TerminalPhase::Failed) {
            return false;
        }
        self.phase = TerminalPhase::Failed;
        self.launch_error = Some(error);
        true
    }
}
