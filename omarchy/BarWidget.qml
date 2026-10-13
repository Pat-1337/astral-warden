import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "pat-1337.gpu-warden"
  ipcTarget: "pat-1337.gpu-warden"

  readonly property string statusPath: "/run/gpu-warden/status.json"
  readonly property string configPath: "/etc/gpu-warden/config.toml"
  readonly property string terminal: "omarchy-launch-floating-terminal-with-presentation "
  readonly property int staleSecs: 5
  readonly property string glyph: ""

  property var status: null
  property bool online: false
  property bool autoUpdate: true
  property bool atBoot: false
  property bool busy: false

  readonly property string level: online ? status.level : "offline"
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  function parse(text) {
    try {
      var next = JSON.parse(text)
      root.status = next
      root.online = Date.now() / 1000 - next.ts < root.staleSecs
    } catch (e) {
      root.online = false
    }
  }

  function parseConfig(text) {
    var match = /^\s*auto_update\s*=\s*(true|false)/m.exec(text)
    root.autoUpdate = !match || match[1] === "true"
  }

  function levelLabel() {
    switch (root.level) {
      case "ok": return "All pins OK"
      case "warn": return "Warning"
      case "crit": return "Critical"
      default: return "Service not running"
    }
  }

  function tooltip() {
    if (root.opened) return ""
    if (!root.online) return "GPU Warden\nService not running"
    return root.status.name + "\n" + root.levelLabel()
  }

  function run(command) {
    root.close()
    if (root.bar) root.bar.run(command)
  }

  function privileged(command) {
    if (root.busy) return
    root.busy = true
    action.command = ["pkexec"].concat(command)
    action.running = true
  }

  function refreshSettings() {
    config.reload()
    if (!boot.running) boot.running = true
  }

  onOpenedChanged: if (opened) refreshSettings()

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

  FileView {
    id: config
    path: root.configPath
    watchChanges: false
    printErrors: false
    onLoaded: root.parseConfig(text())
  }

  Process {
    id: boot
    command: ["systemctl", "is-enabled", "gpu-warden.service"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.atBoot = text.trim() === "enabled"
    }
  }

  Process {
    id: action
    onExited: {
      root.busy = false
      root.refreshSettings()
    }
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
    text: root.online && !(root.bar && root.bar.vertical) ? root.glyph + " " + root.status.text : root.glyph
    active: root.level === "warn" || root.level === "crit"
    dimmed: !root.online
    tooltipText: root.tooltip()
    onPressed: function(b) { root.toggle() }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(14)

        Item {
          width: parent.width
          implicitHeight: Math.max(heroIcon.implicitHeight, heroLabels.implicitHeight, heroAmps.implicitHeight)

          Text {
            id: heroIcon
            text: root.glyph
            color: root.online && root.level !== "ok" ? (root.bar ? root.bar.urgent : Color.urgent) : root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.display
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
          }

          Column {
            id: heroLabels
            anchors.left: heroIcon.right
            anchors.leftMargin: Style.space(14)
            anchors.right: heroAmps.left
            anchors.rightMargin: Style.space(10)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)

            Text {
              text: root.online ? root.status.name : "GPU Warden"
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.title
              font.bold: true
              elide: Text.ElideRight
              width: parent.width
            }

            Text {
              text: root.levelLabel().toUpperCase()
              color: Qt.darker(root.foreground, 1.4)
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.bold: true
              font.letterSpacing: 1.2
              elide: Text.ElideRight
              width: parent.width
            }
          }

          Text {
            id: heroAmps
            text: root.online ? root.status.text : "—"
            color: root.foreground
            font.family: root.fontFamily
            font.pixelSize: Style.font.displayLarge
            font.bold: true
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        Column {
          visible: root.online
          width: parent.width
          spacing: Style.spacing.labelGap

          Repeater {
            model: root.online ? root.status.rows : []

            InfoPair {
              required property var modelData
              label: modelData[0]
              value: modelData[1]
            }
          }
        }

        PanelSeparator { foreground: root.foreground }

        PanelSectionHeader {
          text: "SETTINGS"
          foreground: root.foreground
          fontFamily: root.fontFamily
        }

        Toggle {
          width: parent.width
          label: "Auto update"
          description: "Install new releases when the service starts"
          checked: root.autoUpdate
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.privileged(["gpu-warden", "auto-update", root.autoUpdate ? "off" : "on"])
        }

        Toggle {
          width: parent.width
          label: "Start at boot"
          description: "Run the gpu-warden service at every boot"
          checked: root.atBoot
          foreground: root.foreground
          fontFamily: root.fontFamily
          onClicked: root.privileged(["systemctl", root.atBoot ? "disable" : "enable", "gpu-warden.service"])
        }

        PanelSeparator { foreground: root.foreground }

        Grid {
          id: actions
          width: parent.width
          columns: 3
          spacing: Style.space(6)

          readonly property real cellWidth: (width - spacing * (columns - 1)) / columns

          ActionButton {
            iconText: "󰄪"
            text: "Live view"
            onClicked: root.run(root.terminal + "gpu-warden omarchy watch")
          }

          ActionButton {
            iconText: "󰚰"
            text: "Updates"
            onClicked: root.run("gpu-warden omarchy update")
          }

          ActionButton {
            iconText: "󰑓"
            text: "Restart"
            onClicked: root.privileged(["systemctl", "restart", "gpu-warden.service"])
          }

          ActionButton {
            iconText: "󰒓"
            text: "Config"
            onClicked: root.run(root.terminal + "sudoedit " + root.configPath)
          }

          ActionButton {
            iconText: "󰈙"
            text: "Log"
            onClicked: root.run(root.terminal + "journalctl -u gpu-warden.service -n 200 -f")
          }

          ActionButton {
            iconText: "󰋼"
            text: "About"
            onClicked: root.run("xdg-open https://github.com/Pat-1337/gpu-warden")
          }
        }
      }
    }
  }

  component ActionButton: Button {
    width: actions.cellWidth
    iconSize: Style.font.title
    fontSize: Style.font.bodySmall
    foreground: root.foreground
    fontFamily: root.fontFamily
    horizontalPadding: Style.spacing.controlPaddingX
    verticalPadding: Style.spacing.controlPaddingY + Style.space(2)
    bordered: true
  }

  component InfoPair: Row {
    property string label: ""
    property string value: ""

    width: parent.width
    spacing: Style.space(8)

    InfoLabel { text: label }
    Item { width: Math.max(0, parent.width - parent.children[0].implicitWidth - parent.children[2].implicitWidth - parent.spacing * 2); height: 1 }
    InfoValue { text: value }
  }

  component InfoLabel: Text {
    textFormat: Text.PlainText
    color: root.foreground
    opacity: 0.6
    font.family: root.fontFamily
    font.pixelSize: Style.font.bodySmall
  }

  component InfoValue: Text {
    textFormat: Text.PlainText
    color: root.foreground
    font.family: root.fontFamily
    font.pixelSize: Style.font.bodySmall
  }
}
