//! `github-app.json`: the company's GitHub identity (SDD-RUST §4.1; ADR-0022).
//! The private key is never here: it lives in the secret broker.

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::Slug;

/// `github-app.json`, written by the Architect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GithubApp {
    /// [`GithubApp::VERSION`].
    pub schema_version: u32,
    /// The App's id.
    pub app_id: u64,
    /// The installation on the target account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<u64>,
    /// The App's slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<Slug>,
}

impl Versioned for GithubApp {
    const FILE: &'static str = "github-app.json";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.app_id == 0 {
            return Err(SchemaError::invalid(
                Self::FILE,
                "appId",
                "must be the App's id, a positive number",
            ));
        }
        if self.installation_id == Some(0) {
            return Err(SchemaError::invalid(
                Self::FILE,
                "installationId",
                "must be a positive number",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    #[test]
    fn an_app_parses_round_trips_and_refuses_zero_ids() {
        let app = parse::<GithubApp>(
            r#"{"schemaVersion":1,"appId":42,"installationId":7,"slug":"ephesus-crew"}"#,
        )
        .unwrap();
        assert_eq!(parse::<GithubApp>(&render(&app).unwrap()).unwrap(), app);
        assert!(
            parse::<GithubApp>(r#"{"schemaVersion":1,"appId":0}"#)
                .unwrap_err()
                .to_string()
                .contains("appId")
        );
        assert!(
            parse::<GithubApp>(r#"{"schemaVersion":1,"appId":1,"installationId":0}"#)
                .unwrap_err()
                .to_string()
                .contains("installationId")
        );
        assert!(
            parse::<GithubApp>(r#"{"schemaVersion":1,"appId":1,"slug":"Not A Slug"}"#).is_err()
        );
    }
}
