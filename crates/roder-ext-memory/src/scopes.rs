use roder_api::memory::{MemoryScope, MemoryScopeDescriptor};

pub fn descriptor(scope: MemoryScope) -> MemoryScopeDescriptor {
    let id = scope.stable_id();
    let label = match &scope {
        MemoryScope::Global => "Global memory".to_string(),
        MemoryScope::User(user) => format!("User memory: {user}"),
        MemoryScope::Workspace(workspace) => format!("Workspace memory: {workspace}"),
        MemoryScope::Project(project) => format!("Project memory: {project}"),
        MemoryScope::Session(session) => format!("Session memory: {session}"),
    };
    MemoryScopeDescriptor { id, scope, label }
}

pub fn project(project_id: impl Into<String>) -> MemoryScope {
    MemoryScope::Project(project_id.into())
}
