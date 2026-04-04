use config::{Config, File, FileFormat};

use crate::{config::tui::ThemeConfig, error::FmlError};

pub const DEFAULT_THEME_NAME: &str = "default";

struct BuiltinTheme {
    name: &'static str,
    source: &'static str,
}

const BUILTIN_THEMES: &[BuiltinTheme] = &[
    BuiltinTheme {
        name: "forest",
        source: include_str!("themes/forest.toml"),
    },
    BuiltinTheme {
        name: "kanagawa_dragon",
        source: include_str!("themes/kanagawa_dragon.toml"),
    },
    BuiltinTheme {
        name: "mono",
        source: include_str!("themes/mono.toml"),
    },
    BuiltinTheme {
        name: "ocean",
        source: include_str!("themes/ocean.toml"),
    },
];

pub fn resolve_theme(
    theme_name: &str,
    default_theme: &ThemeConfig,
) -> Result<ThemeConfig, FmlError> {
    let normalized = theme_name.trim().to_ascii_lowercase();

    if normalized == DEFAULT_THEME_NAME {
        return Ok(default_theme.clone());
    }

    let builtin = BUILTIN_THEMES
        .iter()
        .find(|builtin| builtin.name == normalized)
        .ok_or_else(|| {
            FmlError::ThemeError(format!(
                "unknown theme `{theme_name}`; available themes: {}",
                available_theme_names().join(", ")
            ))
        })?;

    parse_theme_source(builtin.name, builtin.source)
}

fn available_theme_names() -> Vec<&'static str> {
    let mut names = vec![DEFAULT_THEME_NAME];
    names.extend(BUILTIN_THEMES.iter().map(|builtin| builtin.name));
    names
}

fn parse_theme_source(theme_name: &str, source: &str) -> Result<ThemeConfig, FmlError> {
    Config::builder()
        .add_source(File::from_str(source, FileFormat::Toml))
        .build()
        .map_err(|error| {
            FmlError::ThemeError(format!(
                "failed to parse built-in theme `{theme_name}`: {error}"
            ))
        })?
        .try_deserialize::<ThemeConfig>()
        .map_err(|error| {
            FmlError::ThemeError(format!(
                "failed to parse built-in theme `{theme_name}`: {error}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn default_theme_name_uses_configured_default_theme() {
        let custom_theme = ThemeConfig {
            query_prompt_fg: Color::LightBlue,
            ..ThemeConfig::default()
        };

        let resolved = resolve_theme(DEFAULT_THEME_NAME, &custom_theme).unwrap();

        assert_eq!(resolved, custom_theme);
    }

    #[test]
    fn built_in_theme_loads_from_embedded_toml() {
        let resolved = resolve_theme("ocean", &ThemeConfig::default()).unwrap();

        assert_eq!(resolved.background, Some(Color::Rgb(0x11, 0x1C, 0x2D)));
        assert_eq!(resolved.query_prompt_fg, Color::LightCyan);
        assert_eq!(resolved.log_match_fg, Color::Cyan);
        assert_eq!(resolved.log_level.info_fg, Color::LightBlue);
    }

    #[test]
    fn theme_lookup_is_case_insensitive() {
        let resolved = resolve_theme("FoReSt", &ThemeConfig::default()).unwrap();

        assert_eq!(resolved.query_prompt_fg, Color::LightGreen);
    }

    #[test]
    fn kanagawa_dragon_theme_loads_expected_palette() {
        let resolved = resolve_theme("kanagawa_dragon", &ThemeConfig::default()).unwrap();

        assert_eq!(resolved.background, Some(Color::Rgb(0x18, 0x16, 0x16)));
        assert_eq!(resolved.query_prompt_fg, Color::Rgb(0x87, 0xA9, 0x87));
        assert_eq!(resolved.log_level.warn_fg, Color::Rgb(0xC4, 0xB2, 0x8A));
    }

    #[test]
    fn unknown_theme_lists_available_builtins() {
        let error = resolve_theme("missing", &ThemeConfig::default()).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("unknown theme `missing`"));
        assert!(message.contains("default, forest, kanagawa_dragon, mono, ocean"));
    }

    #[test]
    fn built_in_parser_accepts_hex_colors() {
        let theme = parse_theme_source(
            "hex-test",
            r##"
background = "#102030"
query_prompt_fg = "#40A0FF"
"##,
        )
        .unwrap();

        assert_eq!(theme.background, Some(Color::Rgb(0x10, 0x20, 0x30)));
        assert_eq!(theme.query_prompt_fg, Color::Rgb(0x40, 0xA0, 0xFF));
    }
}
