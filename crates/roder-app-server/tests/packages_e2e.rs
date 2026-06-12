//! App-server e2e coverage for the `packages/*` JSON-RPC surface (roadmap
//! phase 93). Installs a local fixture package into an isolated
//! RODER_CONFIG_DIR, then drives list/toggle/approve/remove plus
//! update/sync/filters over the protocol, asserting deterministic camelCase
//! JSON fields.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use roder_api::extension::ExtensionRegistryBuilder;
use roder_app_server::{AppServer, AppServerFeatureConfig, LocalAppClient};
use roder_core::fake_provider::FakeInferenceEngine;
use roder_core::{Runtime, RuntimeConfig};
use roder_protocol::{CommandsListResult, JsonRpcRequest, PackagesListResult};
use tokio::sync::Mutex;

/// RODER_CONFIG_DIR is process-global; serialize the tests that override it.
static CONFIG_DIR_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[tokio::test]
async fn packages_install_list_toggle_approve_commands_and_remove_flow() {
    let _guard = CONFIG_DIR_LOCK.lock().await;
    let fixture = TestFixture::new("flow");
    let client = fixture.client();
    let package_root = fixture.package_root.display().to_string();

    let installed = request(
        &client,
        "packages/install",
        Some(serde_json::json!({ "spec": package_root, "scope": "user" })),
    )
    .await;
    let package = &installed["package"];
    assert_eq!(package["packageId"], "pkg-e2e");
    assert_eq!(package["scope"], "user");
    assert_eq!(package["enabled"], true);
    assert_eq!(package["allowScripts"], false);
    assert_eq!(package["extensionsApproved"], false);
    assert_eq!(package["shadowedByProject"], false);
    assert_eq!(
        resource_ids(package),
        vec![
            "pkg-e2e:extension/pkg-e2e-tools",
            "pkg-e2e:skill/getting-started",
            "pkg-e2e:command/pkg-hello",
            "pkg-e2e:theme/pkg-theme",
        ]
    );
    let extension = &package["resources"][0];
    assert_eq!(extension["kind"], "extension");
    assert_eq!(extension["requiresApproval"], true);
    assert_eq!(extension["enabled"], true);
    assert_eq!(installed["diagnostics"], serde_json::json!([]));

    let listed: PackagesListResult = typed_request(&client, "packages/list", None).await;
    assert_eq!(listed.packages.len(), 1);
    assert_eq!(listed.packages[0].record.package_id, "pkg-e2e");
    assert_eq!(listed.packages[0].resources.len(), 4);
    assert!(listed.diagnostics.is_empty(), "{:?}", listed.diagnostics);

    // The package command surfaces through the command registry.
    let commands: CommandsListResult = typed_request(&client, "commands/list", None).await;
    let hello = commands
        .commands
        .iter()
        .find(|command| command.name == "pkg-hello")
        .expect("package command listed");
    assert_eq!(hello.source, "package:pkg-e2e");

    // Disable one resource by id; list reflects it.
    let toggled = request(
        &client,
        "packages/set_enabled",
        Some(serde_json::json!({ "id": "pkg-e2e:command/pkg-hello", "enabled": false })),
    )
    .await;
    assert_eq!(
        toggled["package"]["disabledResources"],
        serde_json::json!(["pkg-e2e:command/pkg-hello"])
    );
    let listed = request(&client, "packages/list", None).await;
    let resources = listed["packages"][0]["resources"]
        .as_array()
        .expect("resources array");
    let command = resources
        .iter()
        .find(|resource| resource["id"] == "pkg-e2e:command/pkg-hello")
        .expect("command resource");
    assert_eq!(command["enabled"], false);
    let skill = resources
        .iter()
        .find(|resource| resource["id"] == "pkg-e2e:skill/getting-started")
        .expect("skill resource");
    assert_eq!(skill["enabled"], true);

    // Approve process extensions; list reflects the approval.
    let approved = request(
        &client,
        "packages/approve_extensions",
        Some(serde_json::json!({ "packageId": "pkg-e2e", "approved": true })),
    )
    .await;
    assert_eq!(approved["package"]["extensionsApproved"], true);
    let listed = request(&client, "packages/list", None).await;
    assert_eq!(listed["packages"][0]["extensionsApproved"], true);

    // Remove; list comes back empty.
    let removed = request(
        &client,
        "packages/remove",
        Some(serde_json::json!({ "specOrId": "pkg-e2e" })),
    )
    .await;
    assert_eq!(removed["removed"]["packageId"], "pkg-e2e");
    let listed: PackagesListResult = typed_request(&client, "packages/list", None).await;
    assert!(listed.packages.is_empty());
}

#[tokio::test]
async fn packages_update_sync_and_set_filters_methods() {
    let _guard = CONFIG_DIR_LOCK.lock().await;
    let fixture = TestFixture::new("update-sync");
    let client = fixture.client();
    let package_root = fixture.package_root.display().to_string();

    let _ = request(
        &client,
        "packages/install",
        Some(serde_json::json!({ "spec": package_root, "scope": "user" })),
    )
    .await;

    // Local-path packages refresh in place on update.
    let updated = request(&client, "packages/update", Some(serde_json::json!({}))).await;
    let outcomes = updated["outcomes"].as_array().expect("update outcomes");
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0]["packageId"], "pkg-e2e");
    assert_eq!(outcomes[0]["scope"], "user");
    assert_eq!(outcomes[0]["status"], "updated");

    // No committed project packages: sync has nothing to materialize.
    let synced = request(&client, "packages/sync", None).await;
    assert_eq!(synced["outcomes"], serde_json::json!([]));

    // `commands: []` filters every command resource out of enumeration.
    let filtered = request(
        &client,
        "packages/set_filters",
        Some(serde_json::json!({
            "packageId": "pkg-e2e",
            "filters": { "commands": [] }
        })),
    )
    .await;
    let resources = filtered["package"]["resources"]
        .as_array()
        .expect("resources array");
    assert!(
        resources
            .iter()
            .all(|resource| resource["kind"] != "command"),
        "{resources:?}"
    );
    assert!(resources.iter().any(|resource| resource["kind"] == "skill"));
}

struct TestFixture {
    config_dir: EnvVarGuard,
    package_root: PathBuf,
    workspace: PathBuf,
}

impl TestFixture {
    fn new(label: &str) -> Self {
        let root = temp_dir(label);
        let config = root.join("config");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let package_root = root.join("pkg-e2e");
        write_fixture_package(&package_root);
        Self {
            config_dir: EnvVarGuard::set("RODER_CONFIG_DIR", &config),
            package_root,
            workspace,
        }
    }

    fn client(&self) -> LocalAppClient {
        let _ = &self.config_dir;
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Arc::new(
            Runtime::new(
                builder.build().unwrap(),
                RuntimeConfig {
                    workspace: Some(self.workspace.display().to_string()),
                    ..RuntimeConfig::default()
                },
            )
            .unwrap(),
        );
        let feature_config = AppServerFeatureConfig::default()
            .with_workspace_registry_path(self.workspace.join("workspaces.json"));
        LocalAppClient::new(Arc::new(AppServer::with_feature_config(
            runtime,
            feature_config,
        )))
    }
}

fn write_fixture_package(root: &Path) {
    write(
        &root.join("roder.toml"),
        r#"[package]
id = "pkg-e2e"
name = "Packages E2E Fixture"
version = "0.1.0"

[resources]
extensions = ["extensions/tools/roder-extension.toml"]
skills = ["skills"]
commands = ["commands"]
themes = ["themes"]
"#,
    );
    write(
        &root.join("skills").join("getting-started").join("SKILL.md"),
        "---\nname: getting-started\ndescription: Fixture skill.\n---\n\nFixture skill body.\n",
    );
    write(
        &root.join("commands").join("pkg-hello.md"),
        "---\ndescription: Greet from the fixture package.\n---\nSay hello to {{arguments}}.\n",
    );
    write(
        &root.join("themes").join("pkg-theme.css"),
        ":root { --accent: #123456; }\n",
    );
    write(
        &root
            .join("extensions")
            .join("tools")
            .join("roder-extension.toml"),
        r#"id = "pkg-e2e-tools"
name = "Pkg E2E Tools"
version = "0.1.0"
api_version = "^0.2"
provides = [{ type = "event_sink", id = "pkg-e2e-sink" }]

[launch]
command = "python3"
args = ["main.py"]
"#,
    );
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roder-packages-e2e-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn resource_ids(package: &serde_json::Value) -> Vec<String> {
    package["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .map(|resource| resource["id"].as_str().expect("resource id").to_string())
        .collect()
}

async fn request(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> serde_json::Value {
    let response = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        })
        .await;
    assert!(
        response.error.is_none(),
        "RPC error for {method}: {:?}",
        response.error
    );
    response.result.unwrap()
}

async fn typed_request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> T {
    serde_json::from_value(request(client, method, params).await).unwrap()
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
