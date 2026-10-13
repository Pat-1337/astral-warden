import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "pat-1337.gpu-warden"

  readonly property string statusPath: "/run/gpu-warden/status.json"
  readonly property int staleSecs: 5
  readonly property string glyph: ""

  property var status: null
  property bool online: false

  readonly property string level: online ? status.level : "offline"

  function parse(text) {
    try {
      var next = JSON.parse(text)
      root.status = next
      root.online = Date.now() / 1000 - next.ts < root.staleSecs
    } catch (e) {
      root.online = false
    }
  }

  function tooltip() {
    if (!root.online) return "GPU Warden\nService not running"

    var lines = [root.status.name]
    for (var i = 0; i < root.status.rows.length; i++) {
      var row = root.status.rows[i]
      lines.push(row[0] + "  " + row[1])
    }

    lines.push("")
    lines.push("Right-click: check for updates")
    return lines.join("\n")
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  FileView {
    id: file
    path: root.statusPath
    watchChanges: false
    printErrors: false
    onLoaded: root.parse(text())
    onLoadFailed: root.online = false
  }

  Timer {
    interval: Math.max(1, root.setting("refreshIntervalSec", 1)) * 1000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: file.reload()
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.online && !root.vertical ? root.glyph + " " + root.status.text : root.glyph
    active: root.level === "warn" || root.level === "crit"
    dimmed: !root.online
    tooltipText: root.tooltip()

    onPressed: function(b) {
      if (!root.bar) return

      if (b === Qt.RightButton) root.bar.run("gpu-warden omarchy update")
      else root.bar.run("omarchy-launch-floating-terminal-with-presentation gpu-warden omarchy watch")
    }
  }
}
