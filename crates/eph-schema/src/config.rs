//! `config.json`: the Architect's settings and the company's mode and consent
//! (SDD-RUST §4.1; ADR-0018 holds the mode here, ADR-0032 the consent).
//!
//! One file with two writers, the Architect by hand and the harness for a mode
//! change or a consent grant, which is why the store writes it only with the
//! Refuse policy: never over a damaged file (inventory P4).

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::check_len;
use crate::time::{Duration, Timestamp};

/// The company's mode (ADR-0018, FR-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The default: autonomous cadences run only on demand.
    Directed,
    /// The Stoa and Gymnasium cadences run on their own (FR-14.4).
    Improving,
}

/// The Architect's answer to "may the company start?" (ADR-0032).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Consent {
    /// When it was given.
    pub granted_at: Timestamp,
    /// The version of the terms consented to; older terms are asked again.
    pub terms: u32,
}

/// Usage-aware pacing (ADR-0023).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Pacing {
    /// Slow down above this share of the window, 1 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_at_percent: Option<u16>,
    /// Hold wakes above this share, 1 to 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_at_percent: Option<u16>,
    /// The gap between wakes when slowed, at most 6 hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_wake_gap: Option<Duration>,
    /// A wake's wall-clock cap, 60 seconds to 6 hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_cap: Option<Duration>,
}

/// The Herald's provider settings, moved here from `prompts/herald/*.md`,
/// because they are configuration and not prose (inventory P12).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Voice {
    /// The provider's model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// The provider's voice id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_id: Option<String>,
}

/// `config.json`. Every field is optional except the version; an absent field
/// means its default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    /// [`Config::VERSION`].
    pub schema_version: u32,
    /// The MemPalace command; absent means `mempalace` on PATH (ADR-0016).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mempalace_command: Option<String>,
    /// The company's mode; absent means [`Mode::Directed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// The Architect's consent; absent means "ask".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<Consent>,
    /// Whether `improving` was ever enabled (FR-14.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ever_enabled_improving: Option<bool>,
    /// Whether the wake word is on (FR-8.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_word_enabled: Option<bool>,
    /// Usage-aware pacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing: Option<Pacing>,
    /// The Herald's provider settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<Voice>,
}

impl Config {
    /// The file as first seeded: the version and nothing else.
    pub fn seed() -> Config {
        Config {
            schema_version: Self::VERSION,
            mempalace_command: None,
            mode: None,
            consent: None,
            ever_enabled_improving: None,
            wake_word_enabled: None,
            pacing: None,
            voice: None,
        }
    }

    /// The mode in force: absent means [`Mode::Directed`].
    pub fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Directed)
    }
}

const SIX_HOURS_MS: i128 = 6 * 60 * 60 * 1000;

impl Versioned for Config {
    const FILE: &'static str = "config.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if let Some(command) = &self.mempalace_command {
            check_len(Self::FILE, "mempalaceCommand", command, 1, 4096)?;
        }
        if let Some(consent) = &self.consent
            && consent.terms == 0
        {
            return Err(SchemaError::invalid(
                Self::FILE,
                "consent.terms",
                "must be at least 1",
            ));
        }
        if let Some(pacing) = &self.pacing {
            let percent = |field: &str, value: Option<u16>, max: u16| match value {
                Some(v) if v == 0 || v > max => Err(SchemaError::invalid(
                    Self::FILE,
                    format!("pacing.{field}"),
                    format!("must be 1 to {max}; it is {v}"),
                )),
                _ => Ok(()),
            };
            percent("slowAtPercent", pacing.slow_at_percent, 100)?;
            percent("holdAtPercent", pacing.hold_at_percent, 200)?;
            if let Some(gap) = pacing.slow_wake_gap
                && gap.as_millis() > SIX_HOURS_MS
            {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    "pacing.slowWakeGap",
                    "must be at most 6h",
                ));
            }
            if let Some(cap) = pacing.wake_cap
                && (cap.as_millis() < 60_000 || cap.as_millis() > SIX_HOURS_MS)
            {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    "pacing.wakeCap",
                    "must be 60s to 6h",
                ));
            }
        }
        if let Some(voice) = &self.voice {
            for (field, value) in [
                ("voice.modelId", &voice.model_id),
                ("voice.voiceId", &voice.voice_id),
            ] {
                if let Some(text) = value {
                    check_len(Self::FILE, field, text, 1, 256)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    #[test]
    fn the_seed_is_the_version_alone_and_means_every_default() {
        let seed = Config::seed();
        assert_eq!(render(&seed).unwrap(), "{\n  \"schemaVersion\": 2\n}\n");
        assert_eq!(seed.mode(), Mode::Directed);
        assert_eq!(parse::<Config>("{\"schemaVersion\":2}").unwrap(), seed);
    }

    #[test]
    fn a_full_config_round_trips() {
        let text = r#"{"schemaVersion":2,"mempalaceCommand":"mempalace","mode":"improving",
            "consent":{"grantedAt":"2026-10-19T12:34:56.789Z","terms":2},"everEnabledImproving":true,
            "wakeWordEnabled":false,"pacing":{"slowAtPercent":80,"holdAtPercent":95,"slowWakeGap":"10m","wakeCap":"2h"},
            "voice":{"modelId":"m","voiceId":"v"}}"#;
        let config = parse::<Config>(text).unwrap();
        assert_eq!(config.mode(), Mode::Improving);
        assert_eq!(parse::<Config>(&render(&config).unwrap()).unwrap(), config);
    }

    #[test]
    fn each_rule_refuses_naming_the_field() {
        let cases = [
            (r#"{"schemaVersion":2,"mode":"busy"}"#, "mode"),
            (
                r#"{"schemaVersion":2,"pacing":{"slowAtPercent":0}}"#,
                "pacing.slowAtPercent",
            ),
            (
                r#"{"schemaVersion":2,"pacing":{"holdAtPercent":201}}"#,
                "pacing.holdAtPercent",
            ),
            (
                r#"{"schemaVersion":2,"pacing":{"wakeCap":"30s"}}"#,
                "pacing.wakeCap",
            ),
            (
                r#"{"schemaVersion":2,"pacing":{"slowWakeGap":"7h"}}"#,
                "pacing.slowWakeGap",
            ),
            (
                r#"{"schemaVersion":2,"consent":{"grantedAt":"2026-10-19T12:34:56.789Z","terms":0}}"#,
                "consent.terms",
            ),
            (
                r#"{"schemaVersion":2,"mempalaceCommand":""}"#,
                "mempalaceCommand",
            ),
            (
                r#"{"schemaVersion":2,"voice":{"modelId":""}}"#,
                "voice.modelId",
            ),
        ];
        for (text, field) in cases {
            let error = parse::<Config>(text).unwrap_err().to_string();
            assert!(
                error.contains(field) || error.contains("unknown variant"),
                "{field}: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_field_or_version_is_refused() {
        let unknown = parse::<Config>(r#"{"schemaVersion":2,"mood":"x"}"#).unwrap_err();
        assert!(matches!(unknown, SchemaError::Shape { .. }), "{unknown}");
        assert!(unknown.to_string().contains("mood"));
        let newer = parse::<Config>(r#"{"schemaVersion":3}"#).unwrap_err();
        assert!(
            newer.to_string().contains("a newer Ephesus wrote it"),
            "{newer}"
        );
        let none = parse::<Config>("{}").unwrap_err();
        assert!(matches!(none, SchemaError::NoVersion { .. }));
        let torn = parse::<Config>("{\"schemaVer").unwrap_err();
        assert!(matches!(torn, SchemaError::NotJson { .. }));
    }

    #[test]
    fn render_refuses_a_value_the_next_read_would_refuse() {
        let mut config = Config::seed();
        config.mempalace_command = Some(String::new());
        assert!(render(&config).is_err());
        config.mempalace_command = None;
        config.schema_version = 9;
        assert!(
            render(&config)
                .unwrap_err()
                .to_string()
                .contains("must be 2, the version")
        );
    }

    /// Round 1 (2026-10-20): every pacing bound was tested past its limit and
    /// none at it.
    #[test]
    fn each_pacing_bound_admits_its_limit() {
        let pacing = |body: &str| format!("{{\"schemaVersion\":2,\"pacing\":{{{body}}}}}");
        for body in [
            "\"slowAtPercent\":100",
            "\"holdAtPercent\":200",
            "\"slowWakeGap\":\"6h\"",
            "\"wakeCap\":\"60s\"",
            "\"wakeCap\":\"6h\"",
        ] {
            assert!(parse::<Config>(&pacing(body)).is_ok(), "{body}");
        }
        for (body, field) in [
            ("\"slowAtPercent\":101", "pacing.slowAtPercent"),
            ("\"wakeCap\":\"7h\"", "pacing.wakeCap"),
            ("\"slowWakeGap\":\"6h 1s\"", "pacing.slowWakeGap"),
        ] {
            let error = parse::<Config>(&pacing(body)).unwrap_err().to_string();
            assert!(error.contains(field), "{body}: {error}");
        }
    }
}
