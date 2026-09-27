import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "oma.smartspaces"
  readonly property var service: bar && bar.shell ? bar.shell.serviceFor("oma.smartspaces") : null
  property int selectedWorkspace: 0
  property var anchorButton: null
  property string actionError: ""
  property string installStatus: ""
  readonly property var selectedState: service ? service.stateFor(selectedWorkspace) : null
  readonly property string selectedLabel: selectedState ? String(selectedState.label || "") : ""
  readonly property string modelRoot: (Quickshell.env("XDG_DATA_HOME") || (Quickshell.env("HOME") + "/.local/share")) + "/oma-smartspaces/models/minilm"
  property bool modelInstalled: false
  readonly property var display: service ? service.displaySettings : ({})
  readonly property bool showIcons: display.show_icons !== false
  readonly property bool showLabels: display.show_labels !== false
  readonly property int iconSize: Math.max(1, Math.min(128, Number(display.icon_size ?? 15)))
  readonly property real strokeWidth: Math.max(0.1, Math.min(8, Number(display.stroke_width ?? 2)))
  readonly property int maxLabelLength: Math.max(1, Math.min(48, Number(display.max_label_length ?? 18)))
  readonly property var iconOptions: [
    { value: "code-2", label: "Code" }, { value: "search", label: "Search" },
    { value: "message-circle", label: "Messages" }, { value: "palette", label: "Design" },
    { value: "music", label: "Music" }, { value: "play", label: "Video" },
    { value: "gamepad-2", label: "Games" }, { value: "folder", label: "Files" },
    { value: "settings", label: "Settings" }, { value: "briefcase-business", label: "Work" },
    { value: "shopping-bag", label: "Shopping" }, { value: "users", label: "People" },
    { value: "circle", label: "Other" }, { value: "house", label: "Home" },
    { value: "database", label: "Database" }, { value: "terminal", label: "Terminal" },
    { value: "file-text", label: "Document" }, { value: "headphones", label: "Headphones" },
    { value: "globe", label: "Web" }, { value: "monitor", label: "Monitor" },
    { value: "pin", label: "Pinned" }
  ]
  readonly property var categoryIcons: ({
    development: "code-2", research: "search", communication: "message-circle",
    design: "palette", music: "music", media: "play", gaming: "gamepad-2",
    files: "folder", system: "settings", office: "briefcase-business",
    shopping: "shopping-bag", social: "users", other: "circle"
  })

  function ids() {
    var result = []
    var values = Hyprland.workspaces.values
    for (var i = 0; i < values.length; ++i) {
      var id = Number(values[i].id)
      if (id !== 0 && result.indexOf(id) === -1) result.push(id)
    }
    var focused = Hyprland.focusedWorkspace
    if (focused && focused.id !== 0 && result.indexOf(focused.id) === -1) result.push(focused.id)
    result.sort(function(a, b) { return a - b })
    return result
  }
  function workspaceById(id) {
    var values = Hyprland.workspaces.values
    for (var i = 0; i < values.length; ++i) if (values[i].id === id) return values[i]
    return null
  }

  function focus(id) {
    var workspace = workspaceById(id)
    if (id < 0 && workspace) workspace.activate()
    else if (bar) bar.run("hyprctl dispatch " + Util.shellQuote("hl.dsp.focus({ workspace = \"" + id + "\" })"))
  }

  function openMenu(id, button) {
    selectedWorkspace = id
    anchorButton = button
    nameField.text = selectedLabel
    iconPicker.value = selectedState ? String(selectedState.icon || "") : ""
    actionError = ""
    menu.open = true
  }

  function apply(type, value) {
    if (!service || !service.command(type, selectedWorkspace, value)) {
      actionError = service ? service.lastError : "Smartspaces service unavailable"
      return
    }
    actionError = ""
  }

  implicitWidth: workspaceRow.implicitWidth
  implicitHeight: workspaceRow.implicitHeight

  RowLayout {
    id: workspaceRow
    anchors.fill: parent
    spacing: Style.space(1)

    Repeater {
      model: root.ids()
      WidgetButton {
        id: button
        required property int modelData
        readonly property var state: root.service ? root.service.stateFor(modelData) : null
        readonly property bool focused: Hyprland.focusedWorkspace && Hyprland.focusedWorkspace.id === modelData
        readonly property var workspace: root.workspaceById(modelData)
        readonly property string fullLabel: state && state.source === "fallback" && modelData < 0 && workspace
          ? String(workspace.name) : (state && state.label ? String(state.label) : (modelData < 0 && workspace ? String(workspace.name) : String(modelData)))
        readonly property string label: fullLabel.length > root.maxLabelLength ? (root.maxLabelLength === 1 ? "…" : fullLabel.slice(0, root.maxLabelLength - 1) + "…") : fullLabel
        readonly property string iconKey: state && state.icon ? String(state.icon) : ""
        readonly property bool showIcon: root.showIcons && state && state.icon && state.icon !== "none"
        readonly property bool showText: root.showLabels || !showIcon

        bar: root.bar
        text: root.vertical ? (modelData < 0 && workspace ? String(workspace.name) : String(modelData)) : label
        fontSize: Style.font.caption
        active: focused
        hasVisualContent: true
        labelVisible: !showIcon && showText
        horizontalMargin: 6
        fixedHeight: root.barSize
        fixedWidth: root.vertical ? root.barSize : -1
        tooltipText: root.service && !root.service.available
          ? "Smartspaces runtime unavailable · right-click to install"
          : "Workspace " + modelData + (state ? " · " + fullLabel + " · " + state.category + " (" + state.source + ")" : "")
        onPressed: function(b) {
          if (b === Qt.RightButton) root.openMenu(modelData, button)
          else root.focus(modelData)
        }

        Row {
          visible: button.showIcon
          anchors.centerIn: parent
          spacing: button.showText ? Style.space(4) : 0
          LucideIcon {
            width: root.iconSize
            height: width
            anchors.verticalCenter: parent.verticalCenter
            name: button.iconKey
            fallbackName: button.state ? (root.categoryIcons[button.state.category] || "circle") : "circle"
            tint: button.focused ? button.activeColor : button.foreground
            strokeWidth: root.strokeWidth
          }
          Text {
            visible: root.showLabels && !root.vertical
            textFormat: Text.PlainText
            text: button.label
            width: Math.min(Style.space(120), implicitWidth)
            elide: Text.ElideRight
            color: button.focused ? button.activeColor : button.foreground
            font.family: button.fontFamily
            font.pixelSize: button.fontSize
            anchors.verticalCenter: parent.verticalCenter
          }
        }
        // WidgetButton normally sizes against its built-in label. Expand for
        // the separate SVG+label row while preserving its click/tooltip API.
        implicitWidth: root.vertical ? root.barSize : (showIcon
          ? root.iconSize + (root.showLabels ? Style.space(4) + Math.min(Style.space(120), labelMeasure.implicitWidth) : 0) + Style.space(12)
          : Math.max(12, labelMeasure.implicitWidth + scaledHorizontalMargin * 2))
        Text {
          id: labelMeasure
          visible: false
          text: button.text
          font.family: button.fontFamily
          font.pixelSize: button.fontSize
        }
      }
    }
  }

  FileView {
    id: modelRevision
    path: root.modelRoot + "/REVISION"
    watchChanges: true
    printErrors: false
    onLoaded: root.modelInstalled = String(text()).trim() !== ""
    onLoadFailed: root.modelInstalled = false
    onFileChanged: reload()
  }
  FileView {
    path: root.modelRoot
    watchChanges: true
    printErrors: false
    onFileChanged: modelRevision.reload()
  }

  Process {
    id: installRuntime
    command: [String(Qt.resolvedUrl("../scripts/install-runtime")).replace(/^file:\/\//, "")]
    onExited: function(code) {
      root.installStatus = code === 0 ? "Runtime installed; reconnecting…" : "Runtime installation failed (exit " + code + ")"
      if (code === 0 && root.service) root.service.reconnect()
    }
    stderr: SplitParser { onRead: function(line) { if (String(line).trim()) root.installStatus = String(line).trim() } }
  }
  Process {
    id: installClassifier
    command: [String(Qt.resolvedUrl("../scripts/install-classifier")).replace(/^file:\/\//, "")]
    onExited: function(code) {
      root.installStatus = code === 0 ? "Classifier installed; restarting local runtime…" : "Classifier installation failed (exit " + code + ")"
      modelRevision.reload()
      if (code === 0 && root.service) root.service.reconnect()
    }
    stderr: SplitParser { onRead: function(line) { if (String(line).trim()) root.installStatus = String(line).trim() } }
  }

  PopupCard {
    id: menu
    anchorItem: root.anchorButton || root
    bar: root.bar
    owner: root
    contentWidth: fittedContentWidth(Style.space(310))
    contentHeight: fittedContentHeight(menuContents.implicitHeight)

    Column {
      id: menuContents
      width: parent.width
      spacing: Style.space(10)

      Text {
        textFormat: Text.PlainText
        text: "Workspace " + root.selectedWorkspace
        color: Color.popups.text
        font.family: Style.font.family
        font.pixelSize: Style.font.heading
        font.bold: true
      }
      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: root.service && root.service.available
          ? (root.selectedState ? "" + root.selectedState.category + " · " + root.selectedState.source : "Classifying…")
          : "Runtime unavailable · numeric workspace display only"
        wrapMode: Text.WordWrap
        color: Color.popups.text
        opacity: 0.75
        font.pixelSize: Style.font.caption
      }
      TextField {
        id: nameField
        width: parent.width
        placeholderText: "Workspace name"
        onAccepted: root.apply("rename", text.trim())
      }
      Button {
        text: "Rename"
        enabled: !!root.service && root.service.available && nameField.text.trim() !== ""
        onClicked: root.apply("rename", nameField.text.trim())
      }
      SearchableDropdown {
        id: iconPicker
        width: parent.width
        label: "Icon"
        options: root.iconOptions
        placeholderText: "Search icons…"
        onChanged: function(value) { root.apply("icon", value) }
      }
      Row {
        spacing: Style.space(5)
        Button {
          text: root.selectedState && root.selectedState.pinned ? "Unpin / auto" : "Pin"
          enabled: !!root.service && root.service.available
          onClicked: root.apply(root.selectedState && root.selectedState.pinned ? "auto" : "pin")
        }
        Button {
          text: "Auto"
          enabled: !!root.service && root.service.available
          onClicked: root.apply("auto")
        }
        Button {
          text: "Reset"
          enabled: !!root.service && root.service.available
          onClicked: root.apply("reset")
        }
      }
      Button {
        text: "Create rule from workspace"
        enabled: !!root.service && root.service.available
        onClicked: root.apply("create-rule")
      }
      Button {
        visible: !root.service || !root.service.available
        text: installRuntime.running ? "Installing runtime…" : "Install runtime"
        enabled: !installRuntime.running
        onClicked: { root.installStatus = "Installing runtime…"; installRuntime.running = true }
      }
      Text {
        textFormat: Text.PlainText
        width: parent.width
        text: "Classifier: " + (root.modelInstalled ? "model files installed" : "model not installed")
        color: Color.popups.text
        font.pixelSize: Style.font.caption
      }
      Button {
        visible: !root.modelInstalled
        text: installClassifier.running ? "Installing classifier…" : "Install classifier"
        enabled: !installClassifier.running
        onClicked: { root.installStatus = "Installing classifier…"; installClassifier.running = true }
      }
      Text {
        visible: root.installStatus !== ""
        width: parent.width
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        text: root.installStatus
        color: Color.popups.text
        font.pixelSize: Style.font.caption
      }
      Text {
        visible: root.actionError !== "" || (!!root.service && root.service.lastError !== "")
        width: parent.width
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        text: root.actionError || (root.service ? root.service.lastError : "")
        color: Color.urgent
        font.pixelSize: Style.font.caption
      }
    }
  }

  function close() { menu.open = false }
  readonly property bool opened: menu.open
}
