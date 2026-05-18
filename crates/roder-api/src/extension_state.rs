use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    events::{ThreadId, TurnId},
    extension::ExtensionId,
};

pub type ExtensionStateKey = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExtensionStoreScope {
    Global,
    Workspace {
        workspace: String,
    },
    Thread {
        thread_id: ThreadId,
    },
    Turn {
        thread_id: ThreadId,
        turn_id: TurnId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionStateRecord {
    pub extension_id: ExtensionId,
    pub key: ExtensionStateKey,
    pub scope: ExtensionStoreScope,
    pub schema_version: u32,
    pub value: serde_json::Value,
}

pub trait ExtensionStateCodec: Send + Sync + 'static {
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;

    fn extension_id(&self) -> ExtensionId;
    fn key(&self) -> ExtensionStateKey;
    fn scope(&self) -> ExtensionStoreScope;
    fn schema_version(&self) -> u32;
    fn migrate_state(
        &self,
        _record: &ExtensionStateRecord,
    ) -> anyhow::Result<Option<ExtensionStateRecord>> {
        Ok(None)
    }

    fn encode_state(&self, state: &Self::State) -> anyhow::Result<ExtensionStateRecord> {
        Ok(ExtensionStateRecord {
            extension_id: self.extension_id(),
            key: self.key(),
            scope: self.scope(),
            schema_version: self.schema_version(),
            value: serde_json::to_value(state)?,
        })
    }

    fn decode_state(&self, record: &ExtensionStateRecord) -> anyhow::Result<Self::State> {
        if record.extension_id != self.extension_id() {
            anyhow::bail!(
                "extension state id mismatch: expected {}, got {}",
                self.extension_id(),
                record.extension_id
            );
        }
        if record.key != self.key() {
            anyhow::bail!(
                "extension state key mismatch: expected {}, got {}",
                self.key(),
                record.key
            );
        }
        if record.scope != self.scope() {
            anyhow::bail!("extension state scope mismatch");
        }
        let record = if record.schema_version == self.schema_version() {
            record.clone()
        } else if let Some(migrated) = self.migrate_state(record)? {
            if migrated.schema_version != self.schema_version() {
                anyhow::bail!(
                    "extension state migration produced schema {}, expected {}",
                    migrated.schema_version,
                    self.schema_version()
                );
            }
            migrated
        } else {
            anyhow::bail!(
                "extension state schema mismatch: expected {}, got {}",
                self.schema_version(),
                record.schema_version
            );
        };
        Ok(serde_json::from_value(record.value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct DemoState {
        value: String,
    }

    struct DemoCodec;

    impl ExtensionStateCodec for DemoCodec {
        type State = DemoState;

        fn extension_id(&self) -> ExtensionId {
            "demo".to_string()
        }

        fn key(&self) -> ExtensionStateKey {
            "state".to_string()
        }

        fn scope(&self) -> ExtensionStoreScope {
            ExtensionStoreScope::Thread {
                thread_id: "thread-a".to_string(),
            }
        }

        fn schema_version(&self) -> u32 {
            1
        }
    }

    #[test]
    fn extension_state_codec_round_trips_thread_scoped_state() {
        let codec = DemoCodec;
        let state = DemoState {
            value: "expanded".to_string(),
        };

        let record = codec.encode_state(&state).unwrap();
        assert_eq!(
            record.scope,
            ExtensionStoreScope::Thread {
                thread_id: "thread-a".to_string()
            }
        );
        assert_eq!(codec.decode_state(&record).unwrap(), state);
    }

    #[test]
    fn extension_state_codec_can_migrate_older_schema() {
        struct MigratingCodec;

        impl ExtensionStateCodec for MigratingCodec {
            type State = DemoState;

            fn extension_id(&self) -> ExtensionId {
                "demo".to_string()
            }

            fn key(&self) -> ExtensionStateKey {
                "state".to_string()
            }

            fn scope(&self) -> ExtensionStoreScope {
                ExtensionStoreScope::Thread {
                    thread_id: "thread-a".to_string(),
                }
            }

            fn schema_version(&self) -> u32 {
                2
            }

            fn migrate_state(
                &self,
                record: &ExtensionStateRecord,
            ) -> anyhow::Result<Option<ExtensionStateRecord>> {
                if record.schema_version != 1 {
                    return Ok(None);
                }
                Ok(Some(ExtensionStateRecord {
                    schema_version: 2,
                    value: serde_json::json!({
                        "value": record.value["legacy_value"],
                    }),
                    ..record.clone()
                }))
            }
        }

        let codec = MigratingCodec;
        let state = codec
            .decode_state(&ExtensionStateRecord {
                extension_id: "demo".to_string(),
                key: "state".to_string(),
                scope: ExtensionStoreScope::Thread {
                    thread_id: "thread-a".to_string(),
                },
                schema_version: 1,
                value: serde_json::json!({ "legacy_value": "expanded" }),
            })
            .unwrap();

        assert_eq!(
            state,
            DemoState {
                value: "expanded".to_string()
            }
        );
    }
}
