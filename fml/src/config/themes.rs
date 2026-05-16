use std::collections::{BTreeMap, HashSet};

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
        name: "kanagawa",
        source: include_str!("themes/kanagawa.toml"),
    },
    BuiltinTheme {
        name: "kanagawa_dragon",
        source: include_str!("themes/kanagawa_dragon.toml"),
    },
    BuiltinTheme {
        name: "gruvbox",
        source: include_str!("themes/gruvbox.toml"),
    },
    BuiltinTheme {
        name: "dracula",
        source: include_str!("themes/dracula.toml"),
    },
    BuiltinTheme {
        name: "catppuccin_mocha",
        source: include_str!("themes/catppuccin_mocha.toml"),
    },
    BuiltinTheme {
        name: "tokyo_night",
        source: include_str!("themes/tokyo_night.toml"),
    },
    BuiltinTheme {
        name: "nord",
        source: include_str!("themes/nord.toml"),
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
    user_themes: &BTreeMap<String, ThemeConfig>,
) -> Result<ThemeConfig, FmlError> {
    let normalized = normalize_theme_name(theme_name);

    if normalized == DEFAULT_THEME_NAME {
        return Ok(default_theme.clone());
    }

    if let Some(builtin) = BUILTIN_THEMES
        .iter()
        .find(|builtin| builtin.name == normalized)
    {
        return parse_theme_source(builtin.name, builtin.source);
    }

    if let Some((_, theme)) = user_themes
        .iter()
        .find(|(name, _)| normalize_theme_name(name) == normalized)
    {
        return Ok(theme.clone());
    }

    Err(FmlError::Theme(format!(
        "unknown theme `{theme_name}`; available themes: {}",
        available_theme_names(user_themes).join(", ")
    )))
}

/// Validate user-defined theme names against built-in and reserved theme names.
pub fn validate_user_themes(user_themes: &BTreeMap<String, ThemeConfig>) -> Result<(), FmlError> {
    let mut seen = HashSet::new();
    for name in user_themes.keys() {
        let normalized = normalize_theme_name(name);
        if normalized == DEFAULT_THEME_NAME
            || BUILTIN_THEMES
                .iter()
                .any(|builtin| builtin.name == normalized)
        {
            return Err(FmlError::Theme(format!(
                "user-defined theme `{name}` collides with a built-in theme name"
            )));
        }
        if !seen.insert(normalized) {
            return Err(FmlError::Theme(format!(
                "user-defined theme `{name}` collides with another user-defined theme name"
            )));
        }
    }
    Ok(())
}

fn available_theme_names(user_themes: &BTreeMap<String, ThemeConfig>) -> Vec<String> {
    let mut names = vec![DEFAULT_THEME_NAME.to_string()];
    names.extend(
        BUILTIN_THEMES
            .iter()
            .map(|builtin| builtin.name.to_string()),
    );
    names.extend(user_themes.keys().cloned());
    names
}

fn normalize_theme_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn parse_theme_source(theme_name: &str, source: &str) -> Result<ThemeConfig, FmlError> {
    Config::builder()
        .add_source(File::from_str(source, FileFormat::Toml))
        .build()
        .map_err(|error| {
            FmlError::Theme(format!(
                "failed to parse built-in theme `{theme_name}`: {error}"
            ))
        })?
        .try_deserialize::<ThemeConfig>()
        .map_err(|error| {
            FmlError::Theme(format!(
                "failed to parse built-in theme `{theme_name}`: {error}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ratatui::style::{Color, Modifier};

    use crate::config::tui::LogMatchStyle;

    use super::*;

    #[test]
    fn default_theme_name_uses_configured_default_theme() {
        let custom_theme = ThemeConfig {
            secondary_accent_fg: Color::LightBlue,
            ..ThemeConfig::default()
        };

        let resolved = resolve_theme(DEFAULT_THEME_NAME, &custom_theme, &BTreeMap::new()).unwrap();

        assert_eq!(resolved, custom_theme);
    }

    #[test]
    fn built_in_theme_loads_from_embedded_toml() {
        let resolved = resolve_theme("ocean", &ThemeConfig::default(), &BTreeMap::new()).unwrap();

        assert_eq!(resolved.background, Some(Color::Rgb(0x11, 0x1C, 0x2D)));
        assert_eq!(resolved.secondary_accent_fg, Color::LightCyan);
        assert_eq!(resolved.log_match_fg, Color::Cyan);
        assert_eq!(resolved.log_level.info_fg, Color::LightBlue);
    }

    #[test]
    fn theme_lookup_is_case_insensitive() {
        let resolved = resolve_theme("FoReSt", &ThemeConfig::default(), &BTreeMap::new()).unwrap();

        assert_eq!(resolved.secondary_accent_fg, Color::LightGreen);
    }

    #[test]
    fn kanagawa_dragon_theme_loads_expected_palette() {
        let resolved =
            resolve_theme("kanagawa_dragon", &ThemeConfig::default(), &BTreeMap::new()).unwrap();

        assert_eq!(resolved.background, Some(Color::Rgb(0x18, 0x16, 0x16)));
        assert_eq!(resolved.secondary_accent_fg, Color::Rgb(0x87, 0xA9, 0x87));
        assert_eq!(resolved.log_level.warn_fg, Color::Rgb(0xC4, 0xB2, 0x8A));
    }

    #[test]
    fn new_built_in_themes_load_expected_palettes() {
        let cases = [
            (
                "kanagawa",
                Color::Rgb(0x1F, 0x1F, 0x28),
                Color::Rgb(0xE6, 0xC3, 0x84),
            ),
            (
                "gruvbox",
                Color::Rgb(0x28, 0x28, 0x28),
                Color::Rgb(0xFA, 0xBD, 0x2F),
            ),
            (
                "dracula",
                Color::Rgb(0x28, 0x2A, 0x36),
                Color::Rgb(0xBD, 0x93, 0xF9),
            ),
            (
                "catppuccin_mocha",
                Color::Rgb(0x1E, 0x1E, 0x2E),
                Color::Rgb(0xCB, 0xA6, 0xF7),
            ),
            (
                "tokyo_night",
                Color::Rgb(0x1A, 0x1B, 0x26),
                Color::Rgb(0x7A, 0xA2, 0xF7),
            ),
            (
                "nord",
                Color::Rgb(0x2E, 0x34, 0x40),
                Color::Rgb(0x88, 0xC0, 0xD0),
            ),
        ];

        for (name, background, accent) in cases {
            let resolved = resolve_theme(name, &ThemeConfig::default(), &BTreeMap::new()).unwrap();

            assert_eq!(resolved.background, Some(background));
            assert_eq!(resolved.primary_accent_fg, accent);
            assert_eq!(resolved.log_selected_modifier, Modifier::BOLD);
            assert_eq!(resolved.log_match_style, LogMatchStyle::Color);
        }
    }

    #[test]
    fn built_in_themes_set_full_theme_config_surface() {
        let required_top_level = [
            "background",
            "border_unfocused_fg",
            "primary_accent_fg",
            "secondary_accent_fg",
            "source_selector_producer_fg",
            "source_selector_group_fg",
            "source_selector_source_fg",
            "log_selected_bg",
            "log_selected_modifier",
            "log_match_fg",
            "log_match_style",
            "log_match_bold",
            "status_dim",
            "log_level",
        ];
        let required_log_level = [
            "default_fg",
            "trace_fg",
            "debug_fg",
            "info_fg",
            "warn_fg",
            "error_fg",
            "fatal_fg",
        ];

        for builtin in BUILTIN_THEMES {
            let value: toml::Value = builtin.source.parse().unwrap();
            let table = value.as_table().unwrap();
            for key in required_top_level {
                assert!(table.contains_key(key), "{} missing {key}", builtin.name);
            }
            let log_level = table
                .get("log_level")
                .and_then(|value| value.as_table())
                .unwrap();
            for key in required_log_level {
                assert!(
                    log_level.contains_key(key),
                    "{} missing log_level.{key}",
                    builtin.name
                );
            }
        }
    }

    #[test]
    fn user_defined_theme_resolves_by_name() {
        let mut user_themes = BTreeMap::new();
        user_themes.insert(
            "solarized".to_string(),
            ThemeConfig {
                primary_accent_fg: Color::Rgb(0xB5, 0x89, 0x00),
                ..ThemeConfig::default()
            },
        );

        let resolved = resolve_theme("solarized", &ThemeConfig::default(), &user_themes).unwrap();

        assert_eq!(resolved.primary_accent_fg, Color::Rgb(0xB5, 0x89, 0x00));
    }

    #[test]
    fn unknown_theme_lists_available_builtins_and_user_themes() {
        let mut user_themes = BTreeMap::new();
        user_themes.insert("solarized".to_string(), ThemeConfig::default());
        let error = resolve_theme("missing", &ThemeConfig::default(), &user_themes).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("unknown theme `missing`"));
        assert!(message.contains("default, forest, kanagawa, kanagawa_dragon"));
        assert!(message.contains("gruvbox"));
        assert!(message.contains("dracula"));
        assert!(message.contains("catppuccin_mocha"));
        assert!(message.contains("tokyo_night"));
        assert!(message.contains("nord"));
        assert!(message.contains("solarized"));
    }

    #[test]
    fn user_defined_theme_cannot_collide_with_builtin_name() {
        let mut user_themes = BTreeMap::new();
        user_themes.insert("Forest".to_string(), ThemeConfig::default());

        let error = validate_user_themes(&user_themes).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("collides with a built-in theme name")
        );
    }

    #[test]
    fn built_in_parser_accepts_hex_colors() {
        let theme = parse_theme_source(
            "hex-test",
            r##"
background = "#102030"
secondary_accent_fg = "#40A0FF"
"##,
        )
        .unwrap();

        assert_eq!(theme.background, Some(Color::Rgb(0x10, 0x20, 0x30)));
        assert_eq!(theme.secondary_accent_fg, Color::Rgb(0x40, 0xA0, 0xFF));
    }
}
