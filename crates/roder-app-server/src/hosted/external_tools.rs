//! One authenticated connection owns execution for a hosted thread.
//! State is tenant-runtime scoped and never contains tool input/output bodies.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use roder_protocol::{
    JsonRpcNotification, ToolExecutionRequestedNotification, ToolExecutionState, ToolExecutorLease,
    ToolsResolveParams,
};
use tokio::sync::Mutex;

const STATE_TTL: Duration = Duration::from_secs(3600);
const MAX_TERMINAL_STATES: usize = 4096;

#[derive(Default)]
pub(crate) struct ExecutorBindings {
    inner: Mutex<Bindings>,
    monitor: OnceLock<tokio::task::AbortHandle>,
}

#[derive(Default)]
struct Bindings {
    generation: u64,
    leases: HashMap<String, Binding>,
    calls: HashMap<String, Call>,
}

struct Binding {
    connection: String,
    lease: ToolExecutorLease,
}

struct Call {
    executor: Option<ToolExecutorLease>,
    state: ToolExecutionState,
    updated: Instant,
}

pub(crate) struct RevokedExecutor {
    pub lease: ToolExecutorLease,
    pub requests: Vec<String>,
}

pub(crate) enum Delivery {
    Send(JsonRpcNotification),
    Suppress,
    Reject(String),
}

impl Bindings {
    fn owns(&self, connection: &str, lease: &ToolExecutorLease) -> bool {
        self.leases
            .get(&lease.thread_id)
            .is_some_and(|binding| binding.connection == connection && binding.lease == *lease)
    }

    fn validate_resolution(
        &self,
        connection: &str,
        params: &ToolsResolveParams,
    ) -> Result<(), &'static str> {
        let lease = params.executor.as_ref().ok_or("executor_required")?;
        if !self.owns(connection, lease) {
            return Err("executor_not_owned");
        }
        let call = self
            .calls
            .get(&params.request_id)
            .ok_or("unknown_execution")?;
        if call.executor.as_ref() != Some(lease)
            || Some(call.state.turn_id.as_str()) != params.turn_id.as_deref()
            || call.state.thread_id != lease.thread_id
            || call.state.state != "pending"
        {
            return Err("execution_not_owned_or_terminal");
        }
        Ok(())
    }

    fn revoke(&mut self, thread: &str, state: &str) -> Option<RevokedExecutor> {
        let binding = self.leases.remove(thread)?;
        let mut requests = Vec::new();
        for (id, call) in &mut self.calls {
            if call.executor.as_ref() == Some(&binding.lease) && call.state.state == "pending" {
                call.state.state = state.to_string();
                call.state.is_error = true;
                call.updated = Instant::now();
                requests.push(id.clone());
            }
        }
        Some(RevokedExecutor {
            lease: binding.lease,
            requests,
        })
    }

    fn prune(&mut self) {
        self.calls
            .retain(|_, call| call.updated.elapsed() < STATE_TTL);
        if self.calls.len() > MAX_TERMINAL_STATES {
            let mut terminal = self
                .calls
                .iter()
                .filter(|(_, call)| call.state.state != "pending")
                .map(|(id, call)| (id.clone(), call.updated))
                .collect::<Vec<_>>();
            terminal.sort_unstable_by_key(|(_, updated)| *updated);
            for (id, _) in terminal
                .into_iter()
                .take(self.calls.len() - MAX_TERMINAL_STATES)
            {
                self.calls.remove(&id);
            }
        }
    }
}

impl Drop for ExecutorBindings {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.get() {
            monitor.abort();
        }
    }
}

impl ExecutorBindings {
    pub async fn bind(
        &self,
        connection: &str,
        thread: &str,
        takeover: bool,
    ) -> Result<(ToolExecutorLease, Option<RevokedExecutor>), &'static str> {
        let mut bindings = self.inner.lock().await;
        if let Some(binding) = bindings.leases.get(thread) {
            if binding.connection == connection {
                return Ok((binding.lease.clone(), None));
            }
            if !takeover {
                return Err("executor_busy");
            }
        }
        let revoked = bindings.revoke(thread, "cancelled");
        bindings.generation += 1;
        let lease = ToolExecutorLease {
            thread_id: thread.to_string(),
            lease_id: uuid::Uuid::new_v4().to_string(),
            generation: bindings.generation,
            contract_version: 1,
        };
        bindings.leases.insert(
            thread.to_string(),
            Binding {
                connection: connection.to_string(),
                lease: lease.clone(),
            },
        );
        Ok((lease, revoked))
    }

    pub async fn unbind(
        &self,
        connection: &str,
        lease: &ToolExecutorLease,
    ) -> Result<Option<RevokedExecutor>, &'static str> {
        let mut bindings = self.inner.lock().await;
        if !bindings.owns(connection, lease) {
            return Err("executor_not_owned");
        }
        Ok(bindings.revoke(&lease.thread_id, "cancelled"))
    }

    pub async fn disconnect(&self, connection: &str) -> Vec<RevokedExecutor> {
        let mut bindings = self.inner.lock().await;
        let threads = bindings
            .leases
            .iter()
            .filter(|(_, b)| b.connection == connection)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        threads
            .iter()
            .filter_map(|id| bindings.revoke(id, "disconnected"))
            .collect()
    }

    pub async fn owns_thread(&self, connection: &str, thread: &str) -> bool {
        self.inner
            .lock()
            .await
            .leases
            .get(thread)
            .is_some_and(|binding| binding.connection == connection)
    }

    #[cfg(test)]
    pub async fn authorize_resolution(
        &self,
        connection: &str,
        params: &ToolsResolveParams,
    ) -> Result<(), &'static str> {
        self.inner
            .lock()
            .await
            .validate_resolution(connection, params)
    }

    pub async fn resolve(
        &self,
        connection: &str,
        params: ToolsResolveParams,
        runtime: &roder_core::Runtime,
    ) -> Result<bool, String> {
        let mut bindings = self.inner.lock().await;
        bindings.validate_resolution(connection, &params)?;
        let resolved = runtime
            .resolve_external_tool_call(
                &params.request_id,
                roder_core::ExternalToolResolution {
                    output: params.output,
                    is_error: params.is_error,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        if let Some(call) = bindings.calls.get_mut(&params.request_id) {
            call.state.state = if resolved { "resolved" } else { "uncertain" }.into();
            call.state.is_error = params.is_error || !resolved;
            call.updated = Instant::now();
        }
        Ok(resolved)
    }

    pub async fn read(
        &self,
        connection: &str,
        lease: &ToolExecutorLease,
        request: &str,
    ) -> Result<Option<ToolExecutionState>, &'static str> {
        let mut bindings = self.inner.lock().await;
        if !bindings.owns(connection, lease) {
            return Err("executor_not_owned");
        }
        bindings.prune();
        Ok(bindings
            .calls
            .get(request)
            .filter(|call| call.state.thread_id == lease.thread_id)
            .map(|call| call.state.clone()))
    }

    pub async fn observe(
        &self,
        connection: &str,
        mut notification: JsonRpcNotification,
    ) -> Delivery {
        if notification.method == "thread/toolExecutionRequested" {
            let Ok(request) = serde_json::from_value::<ToolExecutionRequestedNotification>(
                notification.params.clone(),
            ) else {
                return Delivery::Suppress;
            };
            let mut bindings = self.inner.lock().await;
            bindings.prune();
            let lease = bindings
                .leases
                .get(&request.thread_id)
                .map(|b| b.lease.clone());
            let first = !bindings.calls.contains_key(&request.request_id);
            let call = bindings
                .calls
                .entry(request.request_id.clone())
                .or_insert_with(|| Call {
                    executor: lease.clone(),
                    updated: Instant::now(),
                    state: ToolExecutionState {
                        thread_id: request.thread_id.clone(),
                        turn_id: request.turn_id,
                        request_id: request.request_id.clone(),
                        tool_name: request.call.name,
                        state: if lease.is_some() {
                            "pending"
                        } else {
                            "unavailable"
                        }
                        .to_string(),
                        is_error: lease.is_none(),
                    },
                });
            let owner = call.executor.clone();
            if first && owner.is_none() {
                return Delivery::Reject(request.request_id);
            }
            if call.state.state != "pending" {
                return Delivery::Suppress;
            }
            if let Some(lease) = owner.filter(|lease| bindings.owns(connection, lease)) {
                notification.params["executor"] = serde_json::json!(lease);
                return Delivery::Send(notification);
            }
            return Delivery::Suppress;
        }
        if notification.method == "thread/toolExecutionResolved" {
            let mut bindings = self.inner.lock().await;
            if let Some(call) = notification
                .params
                .get("requestId")
                .and_then(|v| v.as_str())
                .and_then(|id| bindings.calls.get_mut(id))
                && call.state.state == "pending"
            {
                call.state.state = notification
                    .params
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .unwrap_or("resolved")
                    .to_string();
                call.state.is_error = notification
                    .params
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                call.updated = Instant::now();
            }
        }
        Delivery::Send(notification)
    }
}

#[cfg(test)]
mod tests;

mod monitor;
