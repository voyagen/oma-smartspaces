# oma-smartspaces

Local semantic workspace labels for Omarchy's Quickshell bar. The plugin observes Hyprland window metadata, applies explicit overrides and rules before project/application detection, and optionally classifies ambiguous context with a local CPU-only MiniLM model. No workspace data is sent to a network service.

## Install

Install from a Git repository containing this manifest:

```sh
omarchy plugin add <repository-url> --enable
```

Move `oma.smartspaces` into the bar's left section in Omarchy's bar settings and remove the existing workspace widget (`omarchy.workspaces` or another workspace indicator) if you want replacement rather than both. Omarchy does not build third-party Rust binaries during `plugin add`. In the plugin checkout, run:

```sh
./scripts/install-runtime
```

This builds the Rust runtime as an unprivileged user and places the executable in `~/.local/share/oma-smartspaces/runtime/`. If Cargo is missing, the script downloads the official Rust installer, verifies its pinned SHA-256, and installs the toolchain without root or a PATH modification. Rust and Cargo are only needed to build the runtime; neither Python nor Ollama is required.

Automatic application and Git-project labels work without a model. To enable local semantic classification, choose **Install classifier** from the widget menu or run:

```sh
./scripts/install-classifier
```

The installer downloads only the pinned quantized MiniLM ONNX model and tokenizer from Hugging Face, verifies the published model SHA-256 and pinned tokenizer checksums before moving the complete set into `~/.local/share/oma-smartspaces/models/minilm/`. The optional ~23 MB model download is the only network operation during classification setup. After installation, model inference is offline. See `LICENSES/MODEL.txt` and `LICENSES/LUCIDE.txt` for asset notices.

## Commands

`scripts/install-runtime` installs `oma-smartspaces` into `~/.local/bin` (ensure this directory is on PATH) and its Rust backend into `~/.local/share/oma-smartspaces/runtime/`. Use the checkout's `bin/oma-smartspaces` if the bin directory is not on PATH.

```sh
oma-smartspaces status
oma-smartspaces list
oma-smartspaces rename 3 "Research"
oma-smartspaces icon 3 search
oma-smartspaces pin 3
oma-smartspaces auto 3
oma-smartspaces reset 3
oma-smartspaces classify 3
```

Right-click a workspace for Rename, searchable bundled Lucide icons, Pin, Auto, Reset, and Create rule from workspace. The latter derives a Git-project rule when available, otherwise an application rule. Explicit overrides in `~/.config/oma-smartspaces/state.json` take priority over rules and local inference. The widget retains known labels if the runtime exits and shows numbers when no saved state exists.

## Local configuration

Edit `~/.config/oma-smartspaces/config.yaml` (copy `config/defaults.yaml` for a starting point). Config sections control icon/label display, Lucide size and stroke, classifier enablement/confidence, debounce and category hysteresis, fallback, and user rules. The first matching rule wins; `match.git_repo`, `match.app`, and `match.title` may be combined. Rule category names are `development`, `research`, `communication`, `design`, `music`, `media`, `gaming`, `files`, `system`, `office`, `shopping`, `social`, `other`, or a custom lowercase kebab-case name. Corrupt config is reported in the popup and defaults are used without overwriting the file.

Only current application/PID context and resolved states are persisted under `~/.config/oma-smartspaces/`; raw window titles and classification fingerprints remain in memory. Git discovery checks the working directory of same-user window processes for `.git` markers. The plugin never reads page bodies, terminal content, messages, or documents. It does not send workspace metadata to any server. The classifier model is local and only loaded at runtime startup (installing it from the menu reconnects the runtime).

## Development

```sh
cargo test --manifest-path runtime/Cargo.toml
cargo build --release --locked --manifest-path runtime/Cargo.toml
```

Protocol version 1 uses newline-delimited JSON over `oma-smartspaces-runtime serve` stdin/stdout. The widget never waits for the model during workspace switching; it shows the last known state immediately and updates after asynchronous classification.
