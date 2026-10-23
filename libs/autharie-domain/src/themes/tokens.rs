use std::{fmt, ops::RangeInclusive};

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};
use utoipa::ToSchema;

use super::ThemeError;

pub const MAX_TOKENS_BYTES: usize = 32 * 1024;
pub const MAX_FONT_URL_LENGTH: usize = 2048;

const COLOUR_KEYS: [&str; 18] = [
    "primaryButton",
    "primaryButtonLabel",
    "secondaryButton",
    "secondaryButtonLabel",
    "socialButtonBackground",
    "socialButtonLabel",
    "socialButtonBorder",
    "magicLinkButtonBackground",
    "magicLinkButtonLabel",
    "magicLinkButtonBorder",
    "passkeyButtonBackground",
    "passkeyButtonLabel",
    "passkeyButtonBorder",
    "widgetBackground",
    "pageBackground",
    "bodyText",
    "links",
    "error",
];

const BORDER_SIZE_KEYS: [&str; 9] = [
    "buttonRadius",
    "buttonBorderWeight",
    "socialButtonBorderWeight",
    "magicLinkButtonBorderWeight",
    "passkeyButtonBorderWeight",
    "inputRadius",
    "inputBorderWeight",
    "widgetRadius",
    "widgetBorderWeight",
];

const SPACING_KEYS: [&str; 3] = ["widgetPadding", "fieldGap", "sectionGap"];

const FONT_STYLE_KEYS: [&str; 5] = ["title", "subtitle", "body", "buttons", "inputLabels"];

const ROOT_KEYS: [&str; 4] = ["colors", "fonts", "borders", "spacing"];
const FONT_KEYS: [&str; 8] = [
    "url",
    "baseSize",
    "title",
    "subtitle",
    "body",
    "buttons",
    "inputLabels",
    "links",
];

const BORDER_RANGE: RangeInclusive<u64> = 0..=64;
const SPACING_RANGE: RangeInclusive<u64> = 0..=128;
const BASE_SIZE_RANGE: RangeInclusive<u64> = 10..=24;
const WEIGHT_RANGE: RangeInclusive<u64> = 100..=900;
const SIZE_PCT_RANGE: RangeInclusive<f64> = 50.0..=300.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, pattern = "^#[0-9a-fA-F]{6}$", example = "#635dff")]
pub struct HexColour(String);

impl HexColour {
    pub fn parse(value: &str) -> Result<Self, ThemeError> {
        let valid = value.len() == 7
            && value.starts_with('#')
            && value[1..].chars().all(|c| c.is_ascii_hexdigit());

        if !valid {
            return Err(ThemeError::invalid(format!(
                "'{value}' is not a colour: use #rrggbb"
            )));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for HexColour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;

        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThemeShadow {
    None,
    Small,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThemeLinkStyle {
    Normal,
    Underline,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_button: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_button_label: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_button: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_button_label: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_button_background: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_button_label: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_button_border: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_link_button_background: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_link_button_label: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magic_link_button_border: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey_button_background: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey_button_label: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey_button_border: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widget_background: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_background: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_text: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<HexColour>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HexColour>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeFontStyle {
    #[schema(minimum = 100, maximum = 900, multiple_of = 100)]
    pub weight: u16,
    #[schema(minimum = 50, maximum = 300)]
    pub size_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeLinkFont {
    #[schema(minimum = 100, maximum = 900, multiple_of = 100)]
    pub weight: u16,
    #[schema(minimum = 50, maximum = 300)]
    pub size_pct: f64,
    pub style: ThemeLinkStyle,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeFonts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 2048, example = "https://fonts.example.com/inter.css")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 10, maximum = 24)]
    pub base_size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<ThemeFontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<ThemeFontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<ThemeFontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buttons: Option<ThemeFontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_labels: Option<ThemeFontStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<ThemeLinkFont>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeBorders {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub button_radius: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub button_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub social_button_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub magic_link_button_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub passkey_button_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub input_radius: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub input_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub widget_radius: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 64)]
    pub widget_border_weight: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widget_shadow: Option<ThemeShadow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeSpacing {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 128)]
    pub widget_padding: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 128)]
    pub field_gap: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 128)]
    pub section_gap: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ThemeTokens {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<ThemeColors>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fonts: Option<ThemeFonts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<ThemeBorders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<ThemeSpacing>,
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn refuse(path: &str, what: impl fmt::Display) -> ThemeError {
    ThemeError::invalid(format!("{path}: {what}"))
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, ThemeError> {
    value
        .as_object()
        .ok_or_else(|| refuse(path, "must be a JSON object"))
}

fn only_known(map: &Map<String, Value>, path: &str, known: &[&str]) -> Result<(), ThemeError> {
    match map.keys().find(|key| !known.contains(&key.as_str())) {
        Some(unknown) => Err(refuse(&join(path, unknown), "is not a theme token")),
        None => Ok(()),
    }
}

fn integer(value: &Value, path: &str, range: &RangeInclusive<u64>) -> Result<u64, ThemeError> {
    value
        .as_u64()
        .filter(|number| range.contains(number))
        .ok_or_else(|| {
            refuse(
                path,
                format!(
                    "must be a whole number between {} and {}",
                    range.start(),
                    range.end()
                ),
            )
        })
}

fn colour(value: &Value, path: &str) -> Result<(), ThemeError> {
    let text = value
        .as_str()
        .ok_or_else(|| refuse(path, "must be a #rrggbb colour"))?;

    HexColour::parse(text)
        .map(|_| ())
        .map_err(|error| refuse(path, error))
}

fn choice(value: &Value, path: &str, allowed: &[&str]) -> Result<(), ThemeError> {
    match value.as_str() {
        Some(text) if allowed.contains(&text) => Ok(()),
        _ => Err(refuse(
            path,
            format!("must be one of {}", allowed.join(", ")),
        )),
    }
}

fn weight(value: &Value, path: &str) -> Result<(), ThemeError> {
    let number = integer(value, path, &WEIGHT_RANGE)?;

    if number % 100 != 0 {
        return Err(refuse(
            path,
            "must be a weight in steps of 100: 100, 200, ... 900",
        ));
    }

    Ok(())
}

fn size_pct(value: &Value, path: &str) -> Result<(), ThemeError> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && SIZE_PCT_RANGE.contains(number))
        .map(|_| ())
        .ok_or_else(|| {
            refuse(
                path,
                format!(
                    "must be a number between {} and {}",
                    SIZE_PCT_RANGE.start(),
                    SIZE_PCT_RANGE.end()
                ),
            )
        })
}

fn required<'a>(
    map: &'a Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<&'a Value, ThemeError> {
    map.get(key)
        .ok_or_else(|| refuse(&join(path, key), "is required here"))
}

fn font_url(value: &Value, path: &str) -> Result<(), ThemeError> {
    if value.is_null() {
        return Ok(());
    }

    let url = value
        .as_str()
        .ok_or_else(|| refuse(path, "must be an https URL"))?;

    if url.len() > MAX_FONT_URL_LENGTH {
        return Err(refuse(
            path,
            format!("is longer than {MAX_FONT_URL_LENGTH} characters"),
        ));
    }

    let authority = url
        .strip_prefix("https://")
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .filter(|authority| !authority.is_empty() && !authority.contains('@'));
    let clean = !url.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(c, '"' | '\'' | '<' | '>' | '\\' | '`' | '(' | ')')
    });

    if authority.is_none() || !clean {
        return Err(refuse(path, "must be an https URL"));
    }

    Ok(())
}

fn colours(value: &Value) -> Result<(), ThemeError> {
    let map = object(value, "colors")?;
    only_known(map, "colors", &COLOUR_KEYS)?;

    map.iter()
        .try_for_each(|(key, value)| colour(value, &join("colors", key)))
}

fn borders(value: &Value) -> Result<(), ThemeError> {
    let map = object(value, "borders")?;
    let known: Vec<&str> = BORDER_SIZE_KEYS
        .iter()
        .copied()
        .chain(["widgetShadow"])
        .collect();
    only_known(map, "borders", &known)?;

    map.iter().try_for_each(|(key, value)| {
        let path = join("borders", key);

        if key == "widgetShadow" {
            choice(value, &path, &["none", "small", "large"])
        } else {
            integer(value, &path, &BORDER_RANGE).map(|_| ())
        }
    })
}

fn spacing(value: &Value) -> Result<(), ThemeError> {
    let map = object(value, "spacing")?;
    only_known(map, "spacing", &SPACING_KEYS)?;

    map.iter().try_for_each(|(key, value)| {
        integer(value, &join("spacing", key), &SPACING_RANGE).map(|_| ())
    })
}

fn font_style(value: &Value, path: &str, link: bool) -> Result<(), ThemeError> {
    let map = object(value, path)?;
    let known: &[&str] = if link {
        &["weight", "sizePct", "style"]
    } else {
        &["weight", "sizePct"]
    };
    only_known(map, path, known)?;

    weight(required(map, path, "weight")?, &join(path, "weight"))?;
    size_pct(required(map, path, "sizePct")?, &join(path, "sizePct"))?;

    if link {
        choice(
            required(map, path, "style")?,
            &join(path, "style"),
            &["normal", "underline"],
        )?;
    }

    Ok(())
}

fn fonts(value: &Value) -> Result<(), ThemeError> {
    let map = object(value, "fonts")?;
    only_known(map, "fonts", &FONT_KEYS)?;

    map.iter().try_for_each(|(key, value)| {
        let path = join("fonts", key);

        match key.as_str() {
            "url" => font_url(value, &path),
            "baseSize" => integer(value, &path, &BASE_SIZE_RANGE).map(|_| ()),
            "links" => font_style(value, &path, true),
            style if FONT_STYLE_KEYS.contains(&style) => font_style(value, &path, false),
            _ => Ok(()),
        }
    })
}

impl ThemeTokens {
    pub fn parse(value: &Value) -> Result<Self, ThemeError> {
        let size = value.to_string().len();

        if size > MAX_TOKENS_BYTES {
            return Err(refuse(
                "tokens",
                format!("are {size} bytes, more than the {MAX_TOKENS_BYTES} allowed"),
            ));
        }

        let root = object(value, "tokens")?;
        only_known(root, "", &ROOT_KEYS)?;

        for (key, section) in root {
            match key.as_str() {
                "colors" => colours(section)?,
                "fonts" => fonts(section)?,
                "borders" => borders(section)?,
                _ => spacing(section)?,
            }
        }

        Self::deserialize(value).map_err(|error| refuse("tokens", error))
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn refused(value: Value) -> String {
        ThemeTokens::parse(&value).expect_err("refused").to_string()
    }

    fn full() -> Value {
        json!({
            "colors": COLOUR_KEYS.iter().map(|key| (key.to_string(), json!("#a1B2c3"))).collect::<Map<_, _>>(),
            "fonts": {
                "url": "https://fonts.example.com/inter.css?family=Inter:wght@400;600",
                "baseSize": 16,
                "title": { "weight": 600, "sizePct": 150 },
                "subtitle": { "weight": 400, "sizePct": 87.5 },
                "body": { "weight": 400, "sizePct": 87.5 },
                "buttons": { "weight": 600, "sizePct": 100 },
                "inputLabels": { "weight": 500, "sizePct": 100 },
                "links": { "weight": 600, "sizePct": 87.5, "style": "underline" }
            },
            "borders": BORDER_SIZE_KEYS.iter().map(|key| (key.to_string(), json!(3)))
                .chain([("widgetShadow".to_string(), json!("large"))])
                .collect::<Map<_, _>>(),
            "spacing": { "widgetPadding": 24, "fieldGap": 16, "sectionGap": 24 }
        })
    }

    #[test]
    fn nothing_at_all_is_a_valid_theme_and_serialises_to_empty_tokens() {
        let tokens = ThemeTokens::parse(&json!({})).expect("valid");

        assert!(tokens.is_empty());
        assert_eq!(serde_json::to_value(&tokens).unwrap(), json!({}));
    }

    #[test]
    fn every_token_of_ferriskeys_shape_is_accepted_and_survives_the_round_trip() {
        let tokens = ThemeTokens::parse(&full()).expect("valid");

        let back = serde_json::to_value(&tokens).unwrap();
        let mut expected = full();
        expected["fonts"]["title"]["sizePct"] = json!(150.0);
        expected["fonts"]["buttons"]["sizePct"] = json!(100.0);
        expected["fonts"]["inputLabels"]["sizePct"] = json!(100.0);

        assert_eq!(back, expected);
        assert_eq!(ThemeTokens::parse(&back).unwrap(), tokens);
    }

    #[test]
    fn the_key_tables_cover_exactly_the_fields_of_the_types() {
        let tokens = ThemeTokens::parse(&full()).unwrap();
        let value = serde_json::to_value(&tokens).unwrap();
        let keys = |section: &str| -> Vec<String> {
            let mut keys: Vec<String> = value[section]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        };
        let sorted = |table: &[&str]| -> Vec<String> {
            let mut keys: Vec<String> = table.iter().map(|key| key.to_string()).collect();
            keys.sort();
            keys
        };

        assert_eq!(keys("colors"), sorted(&COLOUR_KEYS));
        assert_eq!(keys("spacing"), sorted(&SPACING_KEYS));
        assert_eq!(keys("fonts"), sorted(&FONT_KEYS));
        let border_keys: Vec<&str> = BORDER_SIZE_KEYS
            .iter()
            .copied()
            .chain(["widgetShadow"])
            .collect();
        assert_eq!(keys("borders"), sorted(&border_keys));
        assert_eq!(
            sorted(&ROOT_KEYS),
            vec!["borders", "colors", "fonts", "spacing"]
        );
    }

    #[test]
    fn a_colour_is_exactly_rrggbb() {
        for good in ["#000000", "#ffffff", "#635DFF", "#0a1b2c"] {
            assert!(
                ThemeTokens::parse(&json!({ "colors": { "links": good } })).is_ok(),
                "{good}"
            );
        }

        for bad in [
            "#fff",
            "#12345",
            "#1234567",
            "#12345678",
            "123456",
            "#gggggg",
            "red",
            " #123456",
            "",
        ] {
            let message = refused(json!({ "colors": { "links": bad } }));
            assert!(message.starts_with("colors.links:"), "{bad}: {message}");
        }
        assert!(refused(json!({ "colors": { "links": 3 } })).starts_with("colors.links:"));
        assert!(refused(json!({ "colors": { "links": null } })).starts_with("colors.links:"));
    }

    #[test]
    fn ranges_are_inclusive_and_name_the_field() {
        let ok = |value: Value| assert!(ThemeTokens::parse(&value).is_ok(), "{value}");
        let bad = |value: Value, field: &str| {
            let message = refused(value.clone());
            assert!(
                message.starts_with(&format!("{field}:")),
                "{value}: {message}"
            );
        };

        ok(json!({ "fonts": { "baseSize": 10 } }));
        ok(json!({ "fonts": { "baseSize": 24 } }));
        bad(json!({ "fonts": { "baseSize": 9 } }), "fonts.baseSize");
        bad(json!({ "fonts": { "baseSize": 25 } }), "fonts.baseSize");
        bad(json!({ "fonts": { "baseSize": 16.5 } }), "fonts.baseSize");
        bad(json!({ "fonts": { "baseSize": -1 } }), "fonts.baseSize");

        ok(json!({ "borders": { "buttonRadius": 0 } }));
        ok(json!({ "borders": { "widgetBorderWeight": 64 } }));
        bad(
            json!({ "borders": { "buttonRadius": 65 } }),
            "borders.buttonRadius",
        );
        bad(
            json!({ "borders": { "inputBorderWeight": 100000 } }),
            "borders.inputBorderWeight",
        );
        bad(
            json!({ "borders": { "widgetRadius": "3" } }),
            "borders.widgetRadius",
        );

        ok(json!({ "spacing": { "fieldGap": 128 } }));
        bad(
            json!({ "spacing": { "fieldGap": 129 } }),
            "spacing.fieldGap",
        );
        bad(
            json!({ "spacing": { "sectionGap": -3 } }),
            "spacing.sectionGap",
        );
    }

    #[test]
    fn a_font_style_has_a_weight_in_steps_of_100_and_a_size_between_50_and_300() {
        let style = |weight: Value, size: Value| json!({ "fonts": { "title": { "weight": weight, "sizePct": size } } });

        assert!(ThemeTokens::parse(&style(json!(100), json!(50))).is_ok());
        assert!(ThemeTokens::parse(&style(json!(900), json!(300))).is_ok());
        assert!(refused(style(json!(650), json!(100))).starts_with("fonts.title.weight:"));
        assert!(refused(style(json!(0), json!(100))).starts_with("fonts.title.weight:"));
        assert!(refused(style(json!(1000), json!(100))).starts_with("fonts.title.weight:"));
        assert!(refused(style(json!(400), json!(49.9))).starts_with("fonts.title.sizePct:"));
        assert!(refused(style(json!(400), json!(300.1))).starts_with("fonts.title.sizePct:"));
        assert!(refused(style(json!(400), json!("big"))).starts_with("fonts.title.sizePct:"));
    }

    #[test]
    fn a_font_style_carries_both_its_weight_and_its_size_because_ferriskey_wants_both() {
        assert!(
            refused(json!({ "fonts": { "body": { "weight": 400 } } }))
                .starts_with("fonts.body.sizePct:")
        );
        assert!(
            refused(json!({ "fonts": { "body": { "sizePct": 100 } } }))
                .starts_with("fonts.body.weight:")
        );
        assert!(
            refused(json!({ "fonts": { "links": { "weight": 400, "sizePct": 100 } } }))
                .starts_with("fonts.links.style:")
        );
        assert!(
            refused(
                json!({ "fonts": { "links": { "weight": 400, "sizePct": 100, "style": "wavy" } } })
            )
            .starts_with("fonts.links.style:")
        );
    }

    #[test]
    fn the_shadow_is_none_small_or_large() {
        for good in ["none", "small", "large"] {
            assert!(ThemeTokens::parse(&json!({ "borders": { "widgetShadow": good } })).is_ok());
        }
        assert!(
            refused(json!({ "borders": { "widgetShadow": "huge" } }))
                .starts_with("borders.widgetShadow:")
        );
        assert!(
            refused(json!({ "borders": { "widgetShadow": "Large" } }))
                .starts_with("borders.widgetShadow:")
        );
    }

    #[test]
    fn a_font_url_is_a_short_clean_https_url_or_null() {
        let url = |value: Value| json!({ "fonts": { "url": value } });

        assert!(
            ThemeTokens::parse(&url(json!(
                "https://fonts.googleapis.com/css2?family=Inter:wght@400;700&display=swap"
            )))
            .is_ok()
        );
        assert!(ThemeTokens::parse(&url(Value::Null)).is_ok());

        for bad in [
            "http://fonts.example.com/a.css",
            "//fonts.example.com/a.css",
            "https://",
            "https:///a.css",
            "https://user@fonts.example.com/a.css",
            "https://fonts.example.com/a b.css",
            "https://fonts.example.com/a.css\"onload=1",
            "https://fonts.example.com/a.css);body{display:none",
            "javascript:alert(1)",
            "",
        ] {
            assert!(refused(url(json!(bad))).starts_with("fonts.url:"), "{bad}");
        }
        assert!(refused(url(json!(7))).starts_with("fonts.url:"));

        let long = format!(
            "https://fonts.example.com/{}",
            "a".repeat(MAX_FONT_URL_LENGTH)
        );
        assert!(refused(url(json!(long))).starts_with("fonts.url:"));
        let exact = format!(
            "https://fonts.example.com/{}",
            "a".repeat(MAX_FONT_URL_LENGTH - "https://fonts.example.com/".len())
        );
        assert!(ThemeTokens::parse(&url(json!(exact))).is_ok());
    }

    #[test]
    fn an_unknown_field_is_named_wherever_it_is() {
        assert!(refused(json!({ "page": {} })).starts_with("page:"));
        assert!(
            refused(json!({ "colors": { "primary": "#112233" } })).starts_with("colors.primary:")
        );
        assert!(refused(json!({ "fonts": { "family": "Inter" } })).starts_with("fonts.family:"));
        assert!(refused(json!({ "borders": { "radius": 3 } })).starts_with("borders.radius:"));
        assert!(refused(json!({ "spacing": { "gap": 3 } })).starts_with("spacing.gap:"));
        assert!(
            refused(json!({ "fonts": { "title": { "weight": 400, "sizePct": 100, "color": "#000000" } } }))
                .starts_with("fonts.title.color:")
        );
    }

    #[test]
    fn a_section_and_the_whole_are_objects() {
        for value in [json!([]), json!("x"), json!(null), json!(3)] {
            assert!(refused(value).starts_with("tokens:"));
        }
        assert!(refused(json!({ "colors": [] })).starts_with("colors:"));
        assert!(refused(json!({ "fonts": null })).starts_with("fonts:"));
        assert!(refused(json!({ "fonts": { "title": 3 } })).starts_with("fonts.title:"));
    }

    #[test]
    fn tokens_are_at_most_32_kib() {
        let padding = "x".repeat(MAX_TOKENS_BYTES);
        let message = refused(json!({ "colors": { "links": "#112233" }, "notes": padding }));

        assert!(message.starts_with("tokens:"), "{message}");
        assert!(message.contains("32768"), "{message}");
    }
}
