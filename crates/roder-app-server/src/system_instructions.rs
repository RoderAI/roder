use roder_api::inference::InstructionBundle;

use crate::AppServer;

impl AppServer {
    /// Replace the harness system identity for turns served by this host.
    /// This is host configuration, never a client-controlled request field.
    /// Per-thread developer instructions and runtime overlays remain intact.
    pub fn with_system_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.system_instructions = Some(instructions.into());
        self
    }

    pub(crate) fn turn_instructions(&self) -> InstructionBundle {
        let mut instructions = roder_core::default_instructions();
        if let Some(system) = &self.system_instructions {
            instructions.system = Some(system.clone());
        }
        instructions
    }
}
