import QtQuick
import Quickshell.Io

Item {
  id: root
  property string name: "circle"
  property string fallbackName: "circle"
  property color tint: "white"
  property real strokeWidth: 2
  readonly property var supported: ["code-2", "search", "message-circle", "palette", "music", "play", "gamepad-2", "folder", "settings", "briefcase-business", "shopping-bag", "users", "circle", "house", "database", "terminal", "file-text", "headphones", "globe", "monitor", "pin"]
  readonly property string safeName: supported.indexOf(name) !== -1 ? name
    : (supported.indexOf(fallbackName) !== -1 ? fallbackName : "circle")
  property string svg: ""
  readonly property string resourcePath: String(Qt.resolvedUrl("../resources/lucide/" + safeName + ".svg")).replace(/^file:\/\//, "")
  function hexByte(channel) {
    return ("0" + Math.round(Math.max(0, Math.min(1, channel)) * 255).toString(16)).slice(-2)
  }
  readonly property string tintHex: "#" + hexByte(tint.r) + hexByte(tint.g) + hexByte(tint.b)

  FileView {
    path: root.resourcePath
    printErrors: false
    onLoaded: root.svg = text()
    onLoadFailed: root.svg = ""
  }

  Image {
    anchors.fill: parent
    fillMode: Image.PreserveAspectFit
    smooth: true
    opacity: root.tint.a
    visible: root.svg !== ""
    source: root.svg ? "data:image/svg+xml," + encodeURIComponent(root.svg
      .replace(/currentColor/g, root.tintHex)
      .replace(/stroke-width="2"/g, 'stroke-width="' + root.strokeWidth + '"')) : ""
  }
}
