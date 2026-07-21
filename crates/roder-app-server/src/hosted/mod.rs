//! Hosted multi-tenant gateway (roadmap phase 72, Task 2).
//!
//! Wraps an app-server with tenant/principal authentication, method
//! authorization, per-tenant rate/size limits, gateway-level thread
//! ownership isolation, and audit records — all enforced **before**
//! JSON-RPC dispatch. Local single-user mode never constructs any of
//! this. TLS termination is expected at the load balancer for hosted
//! deployments; the gateway itself speaks WebSocket.

pub mod audit;
pub mod auth;
pub mod authorization;
pub mod gateway;
pub mod hook_delivery;
pub mod hooks;
pub mod rate_limit;
pub mod runtime_pool;
pub mod tenant;

pub use audit::{AuditLog, AuditRecord};
pub use auth::{ExternalBearerVerifier, HostedAuthError, HostedAuthenticator, ServiceAccountKey};
pub use authorization::authorize_method;
pub use gateway::{
    AllowAllHostedRequestPolicy, HostedGatewayController, HostedGatewayOptions,
    HostedRequestPolicy, HostedRequestPolicyDecision, serve_hosted_gateway,
};
pub use hook_delivery::{
    HookDeliveryConfig, HookDeliveryService, HookFailureMode, SIGNATURE_HEADER,
};
pub use hooks::HookStore;
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use runtime_pool::{HostedRuntimePool, HostedRuntimeProfile, TenantAppServerFactory};
pub use tenant::TenantRegistry;
