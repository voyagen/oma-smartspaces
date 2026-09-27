#[path = "../../config/mod.rs"]
mod config;
#[path = "../../classifier/mod.rs"]
mod classifier;

use classifier::LocalClassifier;
use config::{Config, Rule};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{collections::{hash_map::DefaultHasher, BTreeMap, HashMap}, env, fs::{self, DirBuilder, File, OpenOptions}, hash::{Hash, Hasher}, io::{self, BufRead, BufReader, Write}, os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt}, path::{Path, PathBuf}, time::Instant};

const PROTOCOL: u8 = 1;
const MAX_LINE: usize = 1_048_576;

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
struct Workspace {
    id: i32,
    #[serde(default)] apps: Vec<String>,
    #[serde(default)] titles: Vec<String>,
    #[serde(default)] active: String,
    #[serde(default)] pid: Option<u32>,
    #[serde(default)] processes: Vec<Process>,
}
#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
struct Process { pid: u32, #[serde(default)] app: String, #[serde(default)] title: String }
#[derive(Deserialize)]
struct Request {
    protocol: u8,
    #[serde(rename = "type")] kind: String,
    #[serde(default)] workspaces: Vec<Workspace>,
    workspace: Option<i32>,
    value: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct State {
    workspace: i32,
    source: String,
    category: String,
    label: String,
    icon: String,
    confidence: f32,
    pinned: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")] label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] icon: Option<String>,
    #[serde(default)] pinned: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)] overrides: BTreeMap<i32, Override>,
    #[serde(default)] workspaces: Vec<Workspace>,
    #[serde(default)] states: Vec<State>,
}
#[derive(Serialize)]
struct Settings {
    show_icons: bool,
    show_labels: bool,
    max_label_length: usize,
    icon_size: u16,
    stroke_width: f32,
    debounce_ms: u64,
    category_change_delay_ms: u64,
}
impl From<&Config> for Settings {
    fn from(cfg: &Config) -> Self {
        Self {
            show_icons: cfg.display.show_icons,
            show_labels: cfg.display.show_labels,
            max_label_length: cfg.display.max_label_length,
            icon_size: cfg.icons.size,
            stroke_width: cfg.icons.stroke_width,
            debounce_ms: cfg.behavior.debounce_ms,
            category_change_delay_ms: cfg.behavior.category_change_delay_ms,
        }
    }
}
#[derive(Serialize)]
struct States<'a> { protocol: u8, #[serde(rename = "type")] kind: &'static str, workspaces: &'a [State], settings: &'a Settings, #[serde(skip_serializing_if = "Option::is_none")] pending_ms: Option<u64>, #[serde(skip_serializing_if = "String::is_empty")] warning: &'a String }
#[derive(Serialize)]
struct ErrorResponse<'a> { protocol: u8, #[serde(rename = "type")] kind: &'static str, message: &'a str }

struct Runtime {
    config_dir: PathBuf,
    ai: Option<LocalClassifier>,
    ai_cache: HashMap<u64, Option<(String, f32)>>,
    pending: HashMap<i32, (String, Instant)>,
    live: Option<Vec<Workspace>>,
    settings: Settings,
    config_warning: String,
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable).filter(|s| !s.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(fallback))
        .join("oma-smartspaces")
}

fn save_config(path: &Path, cfg: &Config) -> Result<(), String> {
    let text = serde_yaml::to_string(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_file_name(format!(".config.{}.tmp", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        file.write_all(text.as_bytes()).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        File::open(path.parent().ok_or("config path has no parent")?)
            .and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())
    })();
    if result.is_err() { let _ = fs::remove_file(&tmp); }
    result
}

fn locked_store<T>(dir: &Path, operation: impl FnOnce(&mut Stored) -> Result<(T, bool), String>) -> Result<T, String> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock_path = dir.join("state.lock");
    let lock = OpenOptions::new().create(true).read(true).write(true).mode(0o600).open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    lock.lock_exclusive().map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
    let path = dir.join("state.json");
    let mut state: Stored = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Stored::default(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if state.states.iter().any(|s| !config::valid_category(&s.category)
        || !s.confidence.is_finite() || !(0.0..=1.0).contains(&s.confidence)
        || !config::valid_icon(&s.icon)) {
        return Err(format!("{}: invalid category, icon or confidence", path.display()));
    }
    let (result, changed) = operation(&mut state)?;
    if changed {
        let tmp = dir.join(format!(".state.{}.tmp", std::process::id()));
        let written = (|| -> Result<(), String> {
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut file, &state).map_err(|e| e.to_string())?;
            file.write_all(b"\n").map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            File::open(dir).and_then(|f| f.sync_all()).map_err(|e| e.to_string())
        })();
        if written.is_err() { let _ = fs::remove_file(&tmp); }
        written?;
    }
    Ok(result)
}

fn project_for_pid(pid: u32) -> Option<String> {
    if pid == 0 { return None; }
    let process = PathBuf::from(format!("/proc/{pid}"));
    if fs::metadata(&process).ok()?.uid() != unsafe { libc_geteuid() } { return None; }
    let cwd = fs::read_link(process.join("cwd")).ok()?;
    // Never read arbitrary files from a client-supplied path; only inspect Git markers.
    for parent in cwd.ancestors() {
        if parent.join(".git").exists() {
            return parent.file_name().map(|name| name.to_string_lossy().into_owned());
        }
    }
    None
}
// SAFETY: libc getuid has no side effects or preconditions.
unsafe extern "C" { #[link_name = "geteuid"] fn libc_geteuid() -> u32; }

fn chosen_project(workspace: &Workspace) -> Option<String> {
    workspace.pid.and_then(project_for_pid).or_else(|| {
        workspace.processes.iter().filter(|p| p.pid > 0 && (p.app == workspace.active || workspace.apps.contains(&p.app)))
            .find_map(|p| project_for_pid(p.pid))
    })
}

fn match_rule<'a>(rules: &'a [Rule], ws: &Workspace, project: Option<&str>) -> Option<&'a Rule> {
    rules.iter().find(|rule| {
        rule.app().is_none_or(|pattern| ws.apps.iter().chain(std::iter::once(&ws.active)).any(|app| config::matches(pattern, app)))
        && rule.title().is_none_or(|pattern| ws.titles.iter().any(|title| config::matches(pattern, title)))
        && rule.project().is_none_or(|pattern| project.is_some_and(|name| config::matches(pattern, name)))
    })
}

fn builtin(app: &str) -> Option<(&'static str, &'static str)> {
    let app = app.to_ascii_lowercase();
    let app = app.rsplit('/').next().unwrap_or(&app);
    let category = match app {
        "code" | "codium" | "code-oss" | "vscodium" | "jetbrains-idea" | "idea" | "zed" | "neovim" | "nvim" | "emacs" | "kitty" | "alacritty" | "foot" | "wezterm" | "ghostty" | "org.wezfurlong.wezterm" => "development",
        "thunar" | "nautilus" | "org.gnome.nautilus" | "dolphin" | "pcmanfm" => "files",
        "discord" | "slack" | "telegram-desktop" | "signal" | "element" | "thunderbird" => "communication",
        "spotify" | "rhythmbox" | "amberol" | "strawberry" => "music",
        "vlc" | "mpv" | "obs" => "media",
        "gimp" | "inkscape" | "krita" | "figma" | "blender" => "design",
        "libreoffice" => "office",
        "steam" | "heroic" | "lutris" => "gaming",
        "pavucontrol" | "gnome-control-center" | "btop" | "htop" | "mission-center" => "system",
        _ => return None,
    };
    Some((category, config::icon_for(category)))
}

fn abbreviate(s: &str) -> String {
    let word = s.rsplit('/').next().unwrap_or(s).trim();
    if word.is_empty() { return String::new(); }
    let mut label = String::with_capacity(word.len().min(24));
    let mut characters = 0;
    for part in word.split(|c: char| matches!(c, '.' | '-' | '_' | ' ')).filter(|part| !part.is_empty()).take(2) {
        if characters > 0 && characters < 24 { label.push(' '); characters += 1; }
        for ch in part.chars().take(24 - characters) { label.push(ch); characters += 1; }
    }
    label
}

fn category_label(category: &str) -> &str {
    match category {
        "development" => "Development", "research" => "Research",
        "communication" => "Communication", "design" => "Design",
        "music" => "Music", "media" => "Media", "gaming" => "Gaming",
        "files" => "Files", "system" => "System", "office" => "Office",
        "shopping" => "Shopping", "social" => "Social", "other" => "Other",
        _ => category,
    }
}

impl Runtime {
    fn new() -> Self {
        let model_dir = xdg_dir("XDG_DATA_HOME", ".local/share").join("models/minilm");
        let ai = match LocalClassifier::load(&model_dir) {
            Ok(ai) => ai,
            Err(e) => { eprintln!("oma-smartspaces: {e}; using offline deterministic resolver"); None }
        };
        Self { config_dir: xdg_dir("XDG_CONFIG_HOME", ".config"), ai, ai_cache: HashMap::new(), pending: HashMap::new(), live: None, settings: Settings::from(&Config::default()), config_warning: String::new() }
    }

    fn resolve(&mut self, ws: &Workspace, override_: Option<&Override>, cfg: &Config) -> State {
        let project = chosen_project(ws);
        let app = if ws.active.is_empty() { ws.apps.first().map(String::as_str).unwrap_or("") } else { &ws.active };
        let mut state = State { workspace: ws.id, source: "fallback".into(), category: "other".into(), label: ws.id.to_string(), icon: "circle".into(), confidence: 0.0, pinned: override_.is_some_and(|o| o.pinned) };
        if let Some(rule) = match_rule(&cfg.rules, ws, project.as_deref()) {
            state.source = "user-rule".into(); state.category = rule.category.clone();
            state.label = rule.label.clone().unwrap_or_else(|| project.clone().unwrap_or_else(|| abbreviate(app)));
            state.icon = rule.icon.clone().unwrap_or_else(|| config::icon_for(&state.category).into()); state.confidence = 1.0;
        } else if let Some(project) = project {
            state.source = "project".into(); state.category = "development".into();
            state.label = project; state.icon = "code-2".into(); state.confidence = 0.95;
        } else if let Some((category, icon)) = builtin(app).or_else(|| ws.apps.iter().find_map(|app| builtin(app))) {
            state.source = "builtin-rule".into(); state.category = category.into();
            state.label = category_label(category).into(); state.icon = icon.into(); state.confidence = 0.92;
        } else if !app.is_empty() || !ws.titles.is_empty() {
            if cfg.classifier.enabled && self.ai.is_some() {
                let mut fingerprint = DefaultHasher::new();
                app.hash(&mut fingerprint);
                ws.apps.iter().for_each(|application| application.hash(&mut fingerprint));
                ws.titles.iter().take(3).for_each(|title| title.hash(&mut fingerprint));
                cfg.classifier.minimum_confidence.to_bits().hash(&mut fingerprint);
                let key = fingerprint.finish();
                let ai_result = if let Some(cached) = self.ai_cache.get(&key) { cached.clone() }
                    else {
                        let mut text = String::with_capacity(app.len() + ws.apps.iter().map(String::len).sum::<usize>() + ws.titles.iter().take(3).map(String::len).sum::<usize>() + ws.apps.len() + ws.titles.len().min(3) + 14);
                        text.push_str("Active: "); text.push_str(app); text.push_str(" Apps:");
                        for application in &ws.apps { text.push(' '); text.push_str(application); }
                        for title in ws.titles.iter().take(3) { text.push(' '); text.push_str(title); }
                        let contexts = std::iter::once(text.as_str()).chain(ws.titles.iter().take(3).map(String::as_str)).collect::<Vec<_>>();
                        let result = self.ai.as_mut().and_then(|ai| ai.classify(&contexts, cfg.classifier.minimum_confidence));
                        if self.ai_cache.len() >= 512 { self.ai_cache.clear(); }
                        self.ai_cache.insert(key, result.clone()); result
                    };
                if let Some((category, confidence)) = ai_result {
                    state.source = "classifier".into(); state.icon = config::icon_for(&category).into();
                    state.label = category_label(&category).into(); state.category = category; state.confidence = confidence;
                }
            }
            if state.source == "fallback" && !app.is_empty() && cfg.workspace.fallback == "application" {
                state.source = "application".into(); state.label = abbreviate(app);
                state.icon = "circle".into(); state.confidence = 0.55;
            }
        }
        if state.label.is_empty() { state.label = ws.id.to_string(); }
        if let Some(custom) = override_ {
            if custom.label.is_some() || custom.icon.is_some() {
                if let Some(label) = &custom.label { state.label = label.clone(); }
                if let Some(icon) = &custom.icon { state.icon = icon.clone(); }
                state.source = "manual".into(); state.confidence = 1.0;
            }
        }
        state
    }

    fn update_states(&mut self, stored: &mut Stored, cfg: &Config, force: Option<i32>) {
        let live = self.live.take();
        let workspaces = live.as_deref().unwrap_or(&stored.workspaces);
        let old = stored.states.iter().map(|state| (state.workspace, state)).collect::<HashMap<_, _>>();
        let mut next = Vec::with_capacity(workspaces.len());
        for ws in workspaces {
            let custom = stored.overrides.get(&ws.id);
            let mut state = self.resolve(ws, custom, cfg);
            if force != Some(ws.id) && custom.is_some_and(|o| o.pinned) {
                if let Some(previous) = old.get(&ws.id) {
                    state.category = previous.category.clone(); state.label = previous.label.clone();
                    state.icon = previous.icon.clone(); state.source = previous.source.clone(); state.confidence = previous.confidence;
                    if let Some(label) = custom.and_then(|o| o.label.as_ref()) { state.label = label.clone(); state.source = "manual".into(); }
                    if let Some(icon) = custom.and_then(|o| o.icon.as_ref()) { state.icon = icon.clone(); state.source = "manual".into(); }
                }
            } else if let Some(previous) = old.get(&ws.id) {
                if state.category != previous.category
                    && !matches!(state.source.as_str(), "manual" | "user-rule" | "project")
                    && !matches!(previous.source.as_str(), "manual" | "user-rule" | "project")
                    && state.confidence < previous.confidence + cfg.behavior.confidence_margin && force != Some(ws.id) {
                    let pending = self.pending.entry(ws.id).or_insert_with(|| (state.category.clone(), Instant::now()));
                    if pending.0 != state.category { *pending = (state.category.clone(), Instant::now()); }
                    if pending.1.elapsed().as_millis() < u128::from(cfg.behavior.category_change_delay_ms) {
                        state = (**previous).clone(); state.pinned = custom.is_some_and(|o| o.pinned);
                    } else { self.pending.remove(&ws.id); }
                } else { self.pending.remove(&ws.id); }
            }
            if let Some((index, _)) = state.label.char_indices().nth(cfg.display.max_label_length) { state.label.truncate(index); }
            next.push(state);
        }
        self.pending.retain(|id, _| next.iter().any(|state| state.workspace == *id));
        self.live = live;
        stored.states = next;
    }

    fn handle(&mut self, req: Request) -> Result<Vec<State>, String> {
        if req.protocol != PROTOCOL { return Err(format!("unsupported protocol {}", req.protocol)); }
        let cfg = match Config::load(&self.config_dir.join("config.yaml")) {
            Ok(cfg) => { self.config_warning.clear(); cfg }
            Err(error) => {
                self.config_warning = format!("Invalid Smartspaces config: {error}; using defaults");
                eprintln!("oma-smartspaces: {}", self.config_warning);
                Config::default()
            }
        };
        self.settings = Settings::from(&cfg);
        if req.kind == "snapshot" {
            if req.workspaces.len() > 256 { return Err("too many workspaces".into()); }
            let mut workspaces = req.workspaces;
            workspaces.sort_by_key(|w| w.id);
            workspaces.dedup_by_key(|w| w.id);
            for ws in &mut workspaces {
                ws.apps.truncate(128); ws.titles.truncate(128); ws.processes.truncate(128);
                for item in ws.apps.iter_mut().chain(&mut ws.titles).chain(std::iter::once(&mut ws.active)) {
                    while item.len() > 256 { item.pop(); }
                }
            }
            self.live = Some(workspaces.clone());
            for ws in &mut workspaces {
                ws.titles.clear();
                for process in &mut ws.processes { process.title.clear(); }
            }
            let dir = self.config_dir.clone();
            return locked_store(&dir, |stored| {
                let workspaces_changed = stored.workspaces != workspaces;
                let old_states = stored.states.clone();
                stored.workspaces = workspaces;
                self.update_states(stored, &cfg, None);
                let changed = workspaces_changed || old_states != stored.states;
                Ok((stored.states.clone(), changed))
            });
        }
        let id = req.workspace.ok_or("workspace is required")?;
        // Hyprland special workspaces can use negative IDs.
        let dir = self.config_dir.clone();
        locked_store(&dir, |stored| {
            let old_states = stored.states.clone();
            let old_overrides = serde_json::to_vec(&stored.overrides).map_err(|e| e.to_string())?;
            let mut cfg = cfg;
            match req.kind.as_str() {
                "rename" => {
                    let value = req.value.as_deref().ok_or("rename requires value")?.trim();
                    if value.is_empty() || value.chars().count() > 48 || value.chars().any(char::is_control) { return Err("label must contain 1-48 printable characters".into()); }
                    stored.overrides.entry(id).or_default().label = Some(value.into());
                }
                "icon" => {
                    let value = req.value.as_deref().ok_or("icon requires value")?;
                    if !config::valid_icon(value) { return Err("icon must be a Lucide kebab-case name".into()); }
                    stored.overrides.entry(id).or_default().icon = Some(value.into());
                }
                "pin" => stored.overrides.entry(id).or_default().pinned = true,
                "auto" | "reset" => { stored.overrides.remove(&id); self.pending.remove(&id); }
                "classify" => { self.ai_cache.clear(); self.pending.remove(&id); }
                "create-rule" => {
                    let ws = stored.workspaces.iter().find(|w| w.id == id).ok_or("workspace not in latest snapshot")?;
                    let project = chosen_project(ws);
                    let app = if ws.active.trim().is_empty() { ws.apps.first().map(String::as_str).unwrap_or("") } else { ws.active.trim() };
                    if project.is_none() && app.is_empty() { return Err("workspace has no Git project or application".into()); }
                    let resolved = self.resolve(ws, None, &cfg);
                    let new_rule = Rule {
                        matcher: Some(config::Matcher {
                            app: if project.is_none() { Some(app.into()) } else { None },
                            project, title: None,
                        }),
                        app: None, project: None, title: None,
                        category: resolved.category,
                        label: Some(resolved.label),
                        icon: Some(resolved.icon),
                    };
                    // Re-read under the state lock to avoid clobbering another runtime CLI mutation.
                    let path = dir.join("config.yaml");
                    cfg = Config::load(&path)?;
                    self.settings = Settings::from(&cfg);
                    cfg.rules.insert(0, new_rule);
                    save_config(&path, &cfg)?;
                }
                _ => return Err(format!("unknown request type: {}", req.kind)),
            }
            if !stored.workspaces.iter().any(|w| w.id == id) {
                stored.workspaces.push(Workspace { id, ..Workspace::default() });
                stored.workspaces.sort_by_key(|w| w.id);
            }
            self.update_states(stored, &cfg, Some(id));
            let changed = old_overrides != serde_json::to_vec(&stored.overrides).map_err(|e| e.to_string())? || old_states != stored.states;
            Ok((stored.states.clone(), changed))
        })
    }
}

fn response(line: &[u8], runtime: &mut Runtime) -> String {
    let result = serde_json::from_slice::<Request>(line).map_err(|e| e.to_string()).and_then(|req| runtime.handle(req));
    match result {
        Ok(states) => {
            let pending_ms = runtime.pending.values().map(|(_, since)| {
                runtime.settings.category_change_delay_ms
                    .saturating_sub(since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64).max(1)
            }).min();
            serde_json::to_string(&States { protocol: PROTOCOL, kind: "states", workspaces: &states, settings: &runtime.settings, pending_ms, warning: &runtime.config_warning }).unwrap()
        },
        Err(e) => serde_json::to_string(&ErrorResponse { protocol: PROTOCOL, kind: "error", message: &e }).unwrap(),
    }
}

// Consume the entire line on overflow so a bad client cannot desynchronize the stream.
fn read_line_limited(input: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<Option<bool>> {
    let mut saw_bytes = false;
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() { return Ok(saw_bytes.then_some(oversized)); }
        saw_bytes = true;
        let length = buffer.iter().position(|byte| *byte == b'\n').map(|n| n + 1);
        let used = length.unwrap_or(buffer.len());
        if !oversized && line.len().saturating_add(used) > MAX_LINE {
            oversized = true;
            line.clear();
        }
        if !oversized { line.extend_from_slice(&buffer[..used]); }
        input.consume(used);
        if length.is_some() { return Ok(Some(oversized)); }
    }
}

fn serve() -> io::Result<()> {
    let mut runtime = Runtime::new();
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    loop {
        let mut line = Vec::new();
        let oversized = match read_line_limited(&mut input, &mut line)? {
            Some(oversized) => oversized,
            None => return Ok(()),
        };
        let result = if oversized { serde_json::to_string(&ErrorResponse { protocol: PROTOCOL, kind: "error", message: "request exceeds 1 MiB" }).unwrap() }
                     else { response(&line, &mut runtime) };
        writeln!(output, "{result}")?;
        output.flush()?;
    }
}

fn cli() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "status".into());
    if command == "serve" { return serve().map_err(|e| e.to_string()); }
    let mut runtime = Runtime::new();
    if command == "status" || command == "list" {
        if args.next().is_some() { return Err(format!("{command} takes no arguments")); }
        let cfg = Config::load(&runtime.config_dir.join("config.yaml")).unwrap_or_else(|error| {
            eprintln!("oma-smartspaces: invalid config: {error}; using defaults");
            Config::default()
        });
        let dir = runtime.config_dir.clone();
        let states = locked_store(&dir, |stored| {
            runtime.update_states(stored, &cfg, None);
            Ok((stored.states.clone(), false))
        })?;
        if command == "status" { println!("{} workspaces; local model: {}", states.len(), if runtime.ai.is_some() { "ready" } else { "unavailable (offline rules active)" }); }
        for state in states { println!("{}\t{}\t{}\t{}\t{}\t{:.2}{}", state.workspace, state.label, state.icon, state.category, state.source, state.confidence, if state.pinned { "\tpinned" } else { "" }); }
        return Ok(());
    }
    let id: i32 = args.next().ok_or("expected workspace number")?.parse().map_err(|_| "invalid workspace number")?;
    let value = match command.as_str() {
        "rename" | "icon" => Some(args.next().ok_or("expected value")?),
        "pin" | "auto" | "reset" | "classify" | "create-rule" => None,
        _ => return Err(format!("unknown command: {command}; use serve/status/list/rename/icon/pin/auto/reset/classify/create-rule")),
    };
    if args.next().is_some() { return Err("too many arguments".into()); }
    let states = runtime.handle(Request { protocol: PROTOCOL, kind: command, workspaces: vec![], workspace: Some(id), value })?;
    for state in states.into_iter().filter(|s| s.workspace == id) {
        println!("{}\t{}\t{}\t{}\t{}\t{:.2}{}", state.workspace, state.label, state.icon, state.category, state.source, state.confidence, if state.pinned { "\tpinned" } else { "" });
    }
    Ok(())
}

fn main() {
    if let Err(error) = cli() { eprintln!("oma-smartspaces: {error}"); std::process::exit(1); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn isolated() -> (Runtime, PathBuf) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = env::temp_dir().join(format!("oma-smartspaces-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir).unwrap();
        (Runtime { config_dir: dir.clone(), ai: None, ai_cache: HashMap::new(), pending: HashMap::new(), live: None, settings: Settings::from(&Config::default()), config_warning: String::new() }, dir)
    }

    fn request(kind: &str, workspace: i32, value: Option<&str>) -> Request {
        Request { protocol: 1, kind: kind.into(), workspaces: vec![], workspace: Some(workspace), value: value.map(str::to_owned) }
    }
    #[test]
    fn resolver_priority_and_manual_override() {
        let (mut runtime, dir) = isolated();
        let ws = Workspace { id: 4, active: "firefox".into(), apps: vec!["firefox".into()], ..Workspace::default() };
        let cfg = Config { rules: vec![Rule { matcher: None, app: Some("fire*".into()), title: None, project: None, category: "office".into(), label: Some("Research".into()), icon: None }], ..Config::default() };
        let rule = runtime.resolve(&ws, None, &cfg);
        assert_eq!((rule.source.as_str(), rule.category.as_str(), rule.label.as_str()), ("user-rule", "office", "Research"));
        let manual = runtime.resolve(&ws, Some(&Override { label: Some("Mine".into()), icon: None, pinned: false }), &cfg);
        assert_eq!((manual.source.as_str(), manual.label.as_str()), ("manual", "Mine"));
        assert_eq!(runtime.resolve(&ws, None, &Config::default()).source, "application");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn bundled_builtin_labels_and_icons_are_category_specific() {
        let (mut runtime, dir) = isolated();
        for (app, category, label, icon) in [
            ("spotify", "music", "Music", "music"),
            ("steam", "gaming", "Gaming", "gamepad-2"),
        ] {
            let ws = Workspace { id: 1, active: app.into(), ..Workspace::default() };
            let state = runtime.resolve(&ws, None, &Config::default());
            assert_eq!((state.category.as_str(), state.label.as_str(), state.icon.as_str()), (category, label, icon));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_requests_do_not_poison_stream() {
        let (mut runtime, dir) = isolated();
        assert!(response(b"not json", &mut runtime).contains("\"error\""));
        let value = response(br#"{"protocol":1,"type":"snapshot","workspaces":[]}"#, &mut runtime);
        assert!(value.contains("\"states\""));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn configured_label_limit_truncates_displayed_category() {
        let (mut runtime, dir) = isolated();
        fs::write(dir.join("config.yaml"), "display:\n  max_label_length: 3\n").unwrap();
        let reply: serde_json::Value = serde_json::from_str(&response(
            br#"{"protocol":1,"type":"snapshot","workspaces":[{"id":1,"active":"spotify"}]}"#,
            &mut runtime,
        )).unwrap();
        assert_eq!(reply["workspaces"][0]["label"], "Mus");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn yaml_rule_reload_immediately_overrides_application_fallback() {
        let (mut service, dir) = isolated();
        let snapshot = || Request {
            protocol: 1, kind: "snapshot".into(),
            workspaces: vec![Workspace { id: 6, active: "firefox".into(), ..Workspace::default() }],
            workspace: None, value: None,
        };
        assert_eq!(service.handle(snapshot()).unwrap()[0].source, "application");
        fs::write(dir.join("config.yaml"), "rules:\n  - match: { app: firefox }\n    name: Buying\n    category: shopping\n    icon: shopping-bag\n").unwrap();
        let state = service.handle(snapshot()).unwrap().remove(0);
        assert_eq!((state.source.as_str(), state.category.as_str(), state.label.as_str()), ("user-rule", "shopping", "Buying"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cli_mutation_is_visible_to_existing_service_and_reset_restores_rule() {
        let (mut service, dir) = isolated();
        let mut cli = Runtime { config_dir: dir.clone(), ai: None, ai_cache: HashMap::new(), pending: HashMap::new(), live: None, settings: Settings::from(&Config::default()), config_warning: String::new() };
        let snapshot = Request { protocol: 1, kind: "snapshot".into(), workspaces: vec![Workspace { id: 3, active: "spotify".into(), apps: vec!["spotify".into()], ..Workspace::default() }], workspace: None, value: None };
        assert_eq!(service.handle(snapshot).unwrap()[0].category, "music");
        cli.handle(request("rename", 3, Some("Reading"))).unwrap();
        assert_eq!(service.handle(Request { protocol: 1, kind: "snapshot".into(), workspaces: vec![Workspace { id: 3, active: "spotify".into(), ..Workspace::default() }], workspace: None, value: None }).unwrap()[0].label, "Reading");
        service.handle(request("reset", 3, None)).unwrap();
        assert_eq!(cli.handle(request("classify", 3, None)).unwrap()[0].source, "builtin-rule");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_rule_precedes_fallback_and_survives_restart() {
        let (mut service, dir) = isolated();
        service.handle(Request { protocol: 1, kind: "snapshot".into(), workspaces: vec![Workspace { id: 2, active: "firefox".into(), ..Workspace::default() }], workspace: None, value: None }).unwrap();
        let states = service.handle(request("create-rule", 2, None)).unwrap();
        assert_eq!(states[0].source, "user-rule");
        let cfg = Config::load(&dir.join("config.yaml")).unwrap();
        assert_eq!(cfg.rules[0].app(), Some("firefox"));
        let mut restarted = Runtime { config_dir: dir.clone(), ai: None, ai_cache: HashMap::new(), pending: HashMap::new(), live: None, settings: Settings::from(&Config::default()), config_warning: String::new() };
        assert_eq!(restarted.handle(request("classify", 2, None)).unwrap()[0].source, "user-rule");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pinned_state_survives_application_switch_until_auto() {
        let (mut service, dir) = isolated();
        let snapshot = |active: &str| Request {
            protocol: 1, kind: "snapshot".into(),
            workspaces: vec![Workspace { id: 5, active: active.into(), ..Workspace::default() }],
            workspace: None, value: None,
        };
        service.handle(snapshot("spotify")).unwrap();
        service.handle(request("pin", 5, None)).unwrap();
        let pinned = service.handle(snapshot("kitty")).unwrap();
        assert_eq!((pinned[0].category.as_str(), pinned[0].pinned), ("music", true));
        let automatic = service.handle(request("auto", 5, None)).unwrap();
        assert_eq!((automatic[0].category.as_str(), automatic[0].pinned), ("development", false));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stable_category_transition_uses_one_shot_deadline() {
        let (mut runtime, dir) = isolated();
        let snapshot = |app: &str| format!(r#"{{"protocol":1,"type":"snapshot","workspaces":[{{"id":8,"active":"{app}"}}]}}"#);
        let first: serde_json::Value = serde_json::from_str(&response(snapshot("spotify").as_bytes(), &mut runtime)).unwrap();
        assert_eq!(first["workspaces"][0]["category"], "music");
        let pending: serde_json::Value = serde_json::from_str(&response(snapshot("unknown-app").as_bytes(), &mut runtime)).unwrap();
        assert_eq!(pending["workspaces"][0]["category"], "music");
        assert!(pending["pending_ms"].as_u64().is_some_and(|ms| (1..=5000).contains(&ms)));
        runtime.pending.get_mut(&8).unwrap().1 = Instant::now() - std::time::Duration::from_millis(5_100);
        let changed: serde_json::Value = serde_json::from_str(&response(snapshot("unknown-app").as_bytes(), &mut runtime)).unwrap();
        assert_eq!(changed["workspaces"][0]["source"], "application");
        assert!(changed.get("pending_ms").is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn git_project_from_owned_process_beats_builtin() {
        let (mut service, dir) = isolated();
        let project = dir.join("my-project");
        fs::create_dir_all(project.join(".git")).unwrap();
        let mut child = std::process::Command::new("sleep").arg("10").current_dir(&project).spawn().unwrap();
        let snapshot = Request {
            protocol: 1, kind: "snapshot".into(),
            workspaces: vec![Workspace { id: 8, active: "firefox".into(), pid: Some(child.id()), ..Workspace::default() }],
            workspace: None, value: None,
        };
        let result = service.handle(snapshot);
        child.kill().unwrap();
        child.wait().unwrap();
        let state = result.unwrap().remove(0);
        assert_eq!((state.source.as_str(), state.category.as_str(), state.label.as_str()), ("project", "development", "my-project"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn raw_window_titles_are_live_only_not_persisted() {
        let (mut service, dir) = isolated();
        let private_title = "PRIVATE MESSAGE SESSION";
        let snapshot = Request {
            protocol: 1, kind: "snapshot".into(),
            workspaces: vec![Workspace {
                id: 4, active: "unknown-app".into(), titles: vec![private_title.into()],
                processes: vec![Process { pid: 100, app: "unknown-app".into(), title: private_title.into() }],
                ..Workspace::default()
            }],
            workspace: None, value: None,
        };
        service.handle(snapshot).unwrap();
        assert_eq!(service.live.as_ref().unwrap()[0].titles[0], private_title);
        let persisted = fs::read_to_string(dir.join("state.json")).unwrap();
        assert!(!persisted.contains(private_title));
        let stored: Stored = serde_json::from_str(&persisted).unwrap();
        assert!(stored.workspaces[0].titles.is_empty() && stored.workspaces[0].processes[0].title.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_line_is_drained_before_next_request() {
        let input = format!("{}\n{{\"protocol\":1,\"type\":\"snapshot\",\"workspaces\":[]}}\n", "x".repeat(MAX_LINE + 1));
        let mut cursor = BufReader::new(input.as_bytes());
        let mut line = Vec::new();
        assert_eq!(read_line_limited(&mut cursor, &mut line).unwrap(), Some(true));
        assert!(line.is_empty());
        assert_eq!(read_line_limited(&mut cursor, &mut line).unwrap(), Some(false));
        assert!(std::str::from_utf8(&line).unwrap().contains("snapshot"));
    }

    #[test]
    fn invalid_yaml_reports_error_without_destroying_existing_state() {
        let (mut service, dir) = isolated();
        fs::write(dir.join("config.yaml"), "rules: [").unwrap();
        let states = service.handle(request("pin", 7, None)).unwrap();
        assert_eq!(states[0].label, "7");
        assert!(states[0].pinned);
        assert!(service.config_warning.contains("config.yaml"));
        assert_eq!(fs::read_to_string(dir.join("config.yaml")).unwrap(), "rules: [");
        fs::remove_dir_all(dir).unwrap();
    }
}
