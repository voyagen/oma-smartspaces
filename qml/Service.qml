import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Hyprland

Item {
  id: root
  property var shell: null
  property var states: ({})
  property var displaySettings: ({ show_icons: true, show_labels: true, max_label_length: 18,
    icon_size: 15, stroke_width: 2, debounce_ms: 750 })
  property string lastError: ""
  property bool available: false
  property var workspaces: []
  readonly property string configHome: Quickshell.env("XDG_CONFIG_HOME") || (Quickshell.env("HOME") + "/.config")
  readonly property string configDir: configHome + "/oma-smartspaces"
  readonly property string installedExecutable: (Quickshell.env("XDG_DATA_HOME") || (Quickshell.env("HOME") + "/.local/share")) + "/oma-smartspaces/runtime/oma-smartspaces-runtime"
  readonly property string devExecutable: String(Qt.resolvedUrl("../runtime/target/release/oma-smartspaces-runtime")).replace(/^file:\/\//, "")

  function stateFor(id) { return states[String(id)] || null }

  function snapshot() {
    var result = []
    var values = Hyprland.workspaces.values
    for (var i = 0; i < values.length; ++i) {
      var ws = values[i]
      if (!ws || ws.id === 0) continue
      var apps = [], titles = [], processes = []
      var windows = ws.toplevels ? ws.toplevels.values : []
      for (var j = 0; j < windows.length; ++j) {
        var win = windows[j]
        if (!win) continue
        var metadata = win.lastIpcObject || {}
        var app = String(metadata.class || metadata.initialClass || (win.wayland && win.wayland.appId) || "")
        var title = String(win.title || "")
        var pid = Number(metadata.pid || 0)
        if (app) apps.push(app)
        if (title) titles.push(title)
        if (pid > 0) processes.push({ pid: pid, app: app, title: title })
      }
      var focused = Hyprland.activeToplevel
      var active = focused && focused.workspace && focused.workspace.id === ws.id
        ? String((focused.lastIpcObject || {}).class || (focused.wayland && focused.wayland.appId) || "") : ""
      result.push({ id: ws.id, apps: apps, titles: titles, active: active, processes: processes })
    }
    result.sort(function(a, b) { return a.id - b.id })
    workspaces = result
    return result
  }

  function refresh() {
    snapshot()
    if (available) send({ protocol: 1, type: "snapshot", workspaces: workspaces })
  }

  function scheduleRefresh() { debounce.restart() }

  function reconnect() {
    restart.stop()
    if (runtime.running) {
      runtime.running = false
      Qt.callLater(function() { runtime.running = true })
    } else {
      runtime.running = true
    }
  }

  function send(message) {
    if (!available) {
      lastError = "Smartspaces runtime unavailable"
      return false
    }
    runtime.write(JSON.stringify(message) + "\n")
    return true
  }

  function command(type, workspace, value) {
    if (!isFinite(workspace) || workspace === 0) return false
    var message = { protocol: 1, type: type, workspace: workspace }
    if (value !== undefined) message.value = String(value)
    var sent = send(message)
    if (sent) scheduleRefresh()
    return sent
  }

  function receive(line) {
    var response
    try { response = JSON.parse(String(line)) }
    catch (e) { lastError = "Invalid Smartspaces runtime response"; return }
    if (!response || response.protocol !== 1) {
      lastError = "Unsupported Smartspaces protocol"
      return
    }
    if (response.type === "error") {
      lastError = String(response.message || "Smartspaces command failed")
      return
    }
    if (response.type !== "states" || !Array.isArray(response.workspaces)) return
    var next = {}
    // Keep previously classified labels visible until a replacement arrives.
    for (var key in states) next[key] = states[key]
    for (var i = 0; i < response.workspaces.length; ++i) {
      var state = response.workspaces[i]
      if (state && Number(state.workspace) !== 0) next[String(state.workspace)] = state
    }
    if (response.settings) displaySettings = response.settings
    states = next
    lastError = String(response.warning || "")
    if (response.pending_ms !== undefined && isFinite(response.pending_ms)) {
      pendingChange.interval = Math.max(1, Number(response.pending_ms))
      pendingChange.restart()
    } else pendingChange.stop()
  }

  Timer { id: debounce; interval: Math.max(0, Number(root.displaySettings.debounce_ms ?? 750)); onTriggered: root.refresh() }
  Timer { id: pendingChange; repeat: false; onTriggered: root.refresh() }
  Timer { id: restart; interval: 30000; onTriggered: runtime.running = true }

  Connections {
    target: Hyprland
    function onRawEvent(event) {
      if (!event || !event.name) return
      var name = String(event.name)
      if (/workspace|window|monitor/.test(name)) {
        if (/window/.test(name)) Hyprland.refreshToplevels()
        root.scheduleRefresh()
      }
    }
    function onFocusedWorkspaceChanged() { root.scheduleRefresh() }
  }

  FileView {
    path: root.configDir + "/config.yaml"
    watchChanges: true
    printErrors: false
    onFileChanged: { reload(); root.scheduleRefresh() }
    onLoaded: root.scheduleRefresh()
  }
  FileView {
    path: root.configDir + "/state.json"
    watchChanges: true
    printErrors: false
    onFileChanged: { reload(); root.scheduleRefresh() }
    onLoaded: root.scheduleRefresh()
  }
  FileView {
    path: root.configDir
    watchChanges: true
    printErrors: false
    onFileChanged: root.scheduleRefresh()
  }

  Process {
    id: runtime
    // Prefer the rootless installed binary; fall back to a locally built release.
    command: ["sh", "-c", "if [ -x \"$1\" ]; then exec \"$1\" serve; elif [ -x \"$2\" ]; then exec \"$2\" serve; else exit 127; fi",
      "oma-smartspaces", root.installedExecutable, root.devExecutable]
    stdinEnabled: true
    running: true
    onStarted: {
      root.available = true
      root.lastError = ""
      restart.stop()
      root.refresh()
    }
    stdout: SplitParser { onRead: function(line) { root.receive(line) } }
    stderr: SplitParser { onRead: function(line) { if (String(line).trim()) root.lastError = String(line).trim() } }
    onExited: function(code) {
      root.available = false
      root.lastError = "Smartspaces runtime unavailable (exit " + code + ")"
      restart.restart()
    }
  }

  Component.onCompleted: refresh()
}
