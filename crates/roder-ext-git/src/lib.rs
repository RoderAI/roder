mod fork_provider;
mod git;
mod provider;
pub mod worktree;

pub use fork_provider::{GIT_WORKTREE_FORK_PROVIDER_ID, GitWorktreeForkProvider};
pub use provider::{GIT_VCS_PROVIDER_ID, GitExtension, GitProvider};
pub use worktree::{
    GitWorktreeFork, GitWorktreeForkRequest, create_worktree_fork, list_worktree_paths,
    remove_worktree_fork,
};
