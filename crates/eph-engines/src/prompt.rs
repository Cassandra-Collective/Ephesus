//! Render the shipped identity appendix or the Architect's override (invariant 8).

use std::{ffi::OsString, fs, io};

use eph_fs::home::{Condition, prompt};

use crate::{ConfigError, SpawnConfig, config::at};

const IDENTITY_APPENDIX: &str = "engines/identity-appendix.md";
const BUNDLED_IDENTITY: &str = include_str!("../../../prompts/engines/identity-appendix.md");

pub(crate) struct RenderedPrompt(String);

impl RenderedPrompt {
    pub(crate) fn append_argument(self, argv: &mut Vec<OsString>) {
        argv.push(self.0.into());
    }
}

pub(crate) fn identity(
    config: &SpawnConfig<'_>,
) -> Result<(RenderedPrompt, Option<Condition>), ConfigError> {
    let template = prompt(config.prompts_dir, IDENTITY_APPENDIX, BUNDLED_IDENTITY);
    let identity = fs::read_to_string(config.identity_path)
        .map_err(|error| at("read agent identity", config.identity_path, error))?;
    let protocol = fs::read_to_string(config.protocol_path)
        .map_err(|error| at("read Agora protocol", config.protocol_path, error))?;
    let variables = [
        ("agentId", config.agent_id.as_str()),
        ("identity", identity.trim()),
        ("protocol", protocol.trim()),
        ("memory", config.memory),
    ];
    let rendered = render(&template.text, &variables).map_err(|error| {
        at(
            "render identity appendix",
            &config.prompts_dir.join(IDENTITY_APPENDIX),
            error,
        )
    })?;
    Ok((RenderedPrompt(rendered), template.condition))
}

fn render(template: &str, variables: &[(&str, &str)]) -> io::Result<String> {
    let mut remaining = template;
    let mut out = String::new();
    while let Some((before, slot)) = remaining.split_once("{{") {
        out.push_str(before);
        let (name, after) = slot.split_once("}}").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "prompt placeholder has no closing braces",
            )
        })?;
        let value = variables
            .iter()
            .find(|(key, _)| *key == name.trim())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "unknown prompt placeholder")
            })?;
        out.push_str(value.1);
        remaining = after;
    }
    out.push_str(remaining);
    if out.ends_with("\n\n\n") {
        out.truncate(out.trim_end_matches('\n').len());
        out.push('\n');
    }
    Ok(out)
}
