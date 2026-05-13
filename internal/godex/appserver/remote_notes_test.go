package appserver

import "testing"

func TestRemoteRuntimeBoundaryLeavesCloudWorkspaceTLSAndTunnelOrchestrationOutOfScope(t *testing.T) {
	t.Log("remote-runtime is a local appserver entrypoint in this phase; cloud workspace creation, TLS pinning, tunnels, git bootstrap, and machine APIs are intentionally out of scope")
}
