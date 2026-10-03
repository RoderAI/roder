#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysqlSessionConfig {
    pub database_url: String,
    pub tenant_id: String,
    pub max_connections: Option<u32>,
}

impl MysqlSessionConfig {
    pub fn new(
        database_url: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let config = Self {
            database_url: database_url.into(),
            tenant_id: tenant_id.into(),
            max_connections: None,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.database_url.trim().is_empty(),
            "MySQL session database URL is required"
        );
        anyhow::ensure!(
            !self.tenant_id.trim().is_empty(),
            "MySQL session tenant id is required"
        );
        anyhow::ensure!(
            !self.tenant_id.contains('/'),
            "MySQL session tenant id cannot contain '/'"
        );
        Ok(())
    }

    pub fn redacted_database_url(&self) -> String {
        redact_database_url(&self.database_url)
    }
}

/// Validates a tenant id for store scoping (shared by config validation
/// and `for_tenant` handles).
pub fn validate_tenant_id(tenant_id: &str) -> anyhow::Result<String> {
    let tenant_id = tenant_id.trim();
    anyhow::ensure!(!tenant_id.is_empty(), "tenant id is required");
    anyhow::ensure!(
        !tenant_id.contains('/'),
        "tenant id cannot contain '/': {tenant_id}"
    );
    Ok(tenant_id.to_string())
}

pub fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<redacted>".to_string();
    };
    let Some((auth_host, tail)) = rest.split_once('@') else {
        return format!("{scheme}://{rest}");
    };
    let user = auth_host
        .split_once(':')
        .map(|(user, _)| user)
        .unwrap_or(auth_host);
    format!("{scheme}://{user}:<redacted>@{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_config() {
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "tenant").is_ok());
        assert!(MysqlSessionConfig::new("", "tenant").is_err());
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "").is_err());
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "a/b").is_err());
    }

    #[test]
    fn redacts_database_url() {
        assert_eq!(
            redact_database_url("mysql://user:secret@host:3306/db"),
            "mysql://user:<redacted>@host:3306/db"
        );
        assert_eq!(
            redact_database_url("mysql://host:3306/db"),
            "mysql://host:3306/db"
        );
    }
}
