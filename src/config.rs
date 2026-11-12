use std::path::Path;

use crate::error::ApiResult;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub remote: Option<RemoteConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RemoteConfig {
    pub base_url: String,
    pub auth_path: Option<String>,
}

impl AppConfig {
    pub fn from_file(path: impl AsRef<Path>) -> ApiResult<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConfigFile(std::path::PathBuf);
    impl ConfigFile {
        fn new(content: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("che-tauri-config-{}.toml", che_orm::Uuid::now_v7()));
            std::fs::write(&path, content).unwrap();
            Self(path)
        }
    }
    impl Drop for ConfigFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn loads_database_and_optional_remote_configuration() {
        let file = ConfigFile::new(
            r#"
            [database]
            url = 'sqlite://local.sqlite?mode=rwc'
            [remote]
            base_url = "https://example.com/api/"
            auth_path = '/api-token-auth/'
        "#,
        );
        let config = AppConfig::from_file(&file.0).unwrap();
        assert_eq!(config.database.url, "sqlite://local.sqlite?mode=rwc");
        assert_eq!(
            config.remote.unwrap(),
            RemoteConfig {
                base_url: "https://example.com/api/".into(),
                auth_path: Some("/api-token-auth/".into()),
            }
        );
        let file = ConfigFile::new("[database]\nurl = 'C:\\Data\\local.sqlite'\n");
        let config = AppConfig::from_file(&file.0).unwrap();
        assert_eq!(config.database.url, r"C:\Data\local.sqlite");
        assert!(config.remote.is_none());
    }

    #[test]
    fn rejects_missing_fields_and_invalid_configuration_types() {
        for content in [
            "[database]\n",
            "[database]\nurl = 42\n",
            "[database]\nurl = 'db.sqlite'\n[remote]\nbase_url = false\n",
        ] {
            let file = ConfigFile::new(content);
            assert!(AppConfig::from_file(&file.0).is_err());
        }
    }
}
