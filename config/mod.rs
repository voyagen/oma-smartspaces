use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub display: Display,
    #[serde(default)]
    pub icons: Icons,
    #[serde(default)]
    pub classifier: Classifier,
    #[serde(default)]
    pub behavior: Behavior,
    #[serde(default)]
    pub workspace: WorkspaceSettings,
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Display {
    pub show_icons: bool,
    pub show_labels: bool,
    pub max_label_length: usize,
}
impl Default for Display {
    fn default() -> Self { Self { show_icons: true, show_labels: true, max_label_length: 18 } }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Icons {
    pub library: String,
    pub size: u16,
    pub stroke_width: f32,
}
impl Default for Icons {
    fn default() -> Self { Self { library: "lucide".into(), size: 15, stroke_width: 2.0 } }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Classifier {
    pub enabled: bool,
    pub minimum_confidence: f32,
}
impl Default for Classifier {
    fn default() -> Self { Self { enabled: true, minimum_confidence: 0.68 } }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Behavior {
    pub debounce_ms: u64,
    pub category_change_delay_ms: u64,
    pub confidence_margin: f32,
}
impl Default for Behavior {
    fn default() -> Self { Self { debounce_ms: 750, category_change_delay_ms: 5000, confidence_margin: 0.15 } }
}

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceSettings { pub fallback: String }
impl Default for WorkspaceSettings {
    fn default() -> Self { Self { fallback: "application".into() } }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<Matcher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, alias = "git_repo", skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub category: String,
    #[serde(rename = "name", alias = "label", default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Matcher {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "git_repo", alias = "project", default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

impl Rule {
    pub fn app(&self) -> Option<&str> { self.matcher.as_ref().and_then(|m| m.app.as_deref()).or(self.app.as_deref()) }
    pub fn title(&self) -> Option<&str> { self.matcher.as_ref().and_then(|m| m.title.as_deref()).or(self.title.as_deref()) }
    pub fn project(&self) -> Option<&str> { self.matcher.as_ref().and_then(|m| m.project.as_deref()).or(self.project.as_deref()) }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let config: Self = serde_yaml::from_str(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !(1..=48).contains(&config.display.max_label_length)
            || !config.classifier.minimum_confidence.is_finite()
            || !(0.0..=1.0).contains(&config.classifier.minimum_confidence)
            || !config.behavior.confidence_margin.is_finite()
            || !(0.0..=1.0).contains(&config.behavior.confidence_margin)
            || config.behavior.category_change_delay_ms > 60_000
            || config.behavior.debounce_ms > 60_000
            || config.icons.library != "lucide"
            || !(1..=128).contains(&config.icons.size)
            || !config.icons.stroke_width.is_finite()
            || !(0.1..=8.0).contains(&config.icons.stroke_width)
            || !matches!(config.workspace.fallback.as_str(), "application" | "number") {
            return Err(format!("{}: invalid display, classifier, behavior or workspace setting", path.display()));
        }
        for (index, rule) in config.rules.iter().enumerate() {
            if !valid_category(&rule.category)
                || [rule.app(), rule.title(), rule.project()].iter().all(|value| value.is_none())
                || [rule.app(), rule.title(), rule.project()].iter().flatten().any(|value| value.trim().is_empty()) {
                return Err(format!("{}: rule {} requires a nonempty matcher and valid category", path.display(), index + 1));
            }
            if rule.icon.as_deref().is_some_and(|icon| !valid_icon(icon)) {
                return Err(format!("{}: rule {} has invalid icon", path.display(), index + 1));
            }
        }
        Ok(config)
    }
}

pub const CATEGORIES: &[(&str, &str)] = &[
    ("development", "code-2"), ("research", "search"), ("communication", "message-circle"),
    ("design", "palette"), ("music", "music"), ("media", "play"),
    ("gaming", "gamepad-2"), ("files", "folder"), ("system", "settings"),
    ("office", "briefcase-business"), ("shopping", "shopping-bag"), ("social", "users"),
    ("other", "circle"),
];

pub fn valid_category(s: &str) -> bool { valid_icon(s) }

pub fn icon_for(category: &str) -> &'static str {
    CATEGORIES.iter().find(|(name, _)| *name == category).map_or("circle", |(_, icon)| icon)
}

pub fn valid_icon(icon: &str) -> bool {
    !icon.is_empty() && icon.len() <= 64 && icon.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

// Case-insensitive substring by default, '*' and '?' enable simple anchored glob patterns.
pub fn matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();
    if !pattern.contains('*') && !pattern.contains('?') { return value.contains(&pattern); }
    let (p, v) = (pattern.as_bytes(), value.as_bytes());
    let (mut i, mut j, mut star, mut backtrack) = (0, 0, None, 0);
    while j < v.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == v[j]) { i += 1; j += 1; }
        else if i < p.len() && p[i] == b'*' { star = Some(i); i += 1; backtrack = j; }
        else if let Some(s) = star { i = s + 1; backtrack += 1; j = backtrack; }
        else { return false; }
    }
    while i < p.len() && p[i] == b'*' { i += 1; }
    i == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glob_and_substring_rules() {
        assert!(matches("code", "Visual Studio Code"));
        assert!(matches("*terminal*", "Alacritty Terminal"));
        assert!(!matches("foo*", "barfoo"));
        assert!(matches("*.rs", "main.rs"));
    }
    #[test]
    fn documented_project_rule_roundtrips() {
        let text = "rules:\n  - match: { git_repo: oma-smartspaces }\n    name: Smartspaces\n    category: development\n    icon: house\n";
        let cfg: Config = serde_yaml::from_str(text).unwrap();
        assert_eq!(cfg.rules[0].project(), Some("oma-smartspaces"));
        assert_eq!(cfg.rules[0].label.as_deref(), Some("Smartspaces"));
        let written = serde_yaml::to_string(&cfg).unwrap();
        let again: Config = serde_yaml::from_str(&written).unwrap();
        assert_eq!(again.rules[0].project(), Some("oma-smartspaces"));
    }
}
