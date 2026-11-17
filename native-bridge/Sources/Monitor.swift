import Foundation
import AppKit
import CoreGraphics
import AudioToolbox

final class FeedbackPlayer {
    private var tickSound: NSSound?

    init(wavPath: String? = nil) {
        if let wavPath = wavPath, FileManager.default.fileExists(atPath: wavPath) {
            tickSound = NSSound(contentsOfFile: wavPath, byReference: true)
        }
    }

    func updateWavPath(_ path: String?) {
        guard let path = path, FileManager.default.fileExists(atPath: path) else { return }
        tickSound = NSSound(contentsOfFile: path, byReference: true)
    }

    func play(kind: String) {
        switch kind.lowercased() {
        case "tick":
            if let s = tickSound {
                s.stop()
                s.currentTime = 0
                s.play()
            } else {
                AudioServicesPlaySystemSound(1104)
            }
            NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        case "alignment":
            NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        case "level":
            NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
        case "generic":
            NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
        case "success":
            NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
        case "error":
            NSSound.beep()
        default:
            break
        }
    }
}

final class MonitorController {
    static let shared = MonitorController()

    private var dragKeyCode: UInt16 = 125 // Down Arrow
    private var dragModifierOnly: String? = nil
    private var idleShortcut: ParsedShortcut? = ParsedShortcut.parse("Option+Shift+Space")
    private var mouseTriggerButton: Int64? = nil // e.g. 2 = middle, 3/4 = side buttons
    private var wheelOpen = false

    private let dragPasteboard = NSPasteboard(name: .drag)
    private var lastChangeCount: Int = -1
    private var dragActive = false
    private var dragFiles: [String] = []
    private var dragKeyHeld = false
    private var dragKeyConsumed = false
    private var mouseTriggerHeld = false

    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var fallbackMonitor: Any?
    private var currentAxTrusted = false
    private var axPollTick = 0
    private var dragTimer: Timer?
    private var feedback = FeedbackPlayer()

    func configure(dragKey: String?, idleShortcutSpec: String?, mouseButton: Int?, wavPath: String?, isWheelOpen: Bool?) {
        if let dk = dragKey {
            let low = dk.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["option", "alt", "control", "ctrl", "shift", "command", "cmd", "fn", "function", "globe"].contains(low) {
                dragModifierOnly = (low == "function" || low == "globe") ? "fn" : low
                if dragModifierOnly == "fn" {
                    dragKeyCode = 63
                }
            } else {
                dragModifierOnly = nil
                if let kc = keyCodeForName(dk) {
                    dragKeyCode = kc
                }
            }
        }
        if let idle = idleShortcutSpec {
            idleShortcut = ParsedShortcut.parse(idle)
        }
        if let mb = mouseButton {
            mouseTriggerButton = mb >= 2 ? Int64(mb) : nil
        }
        if let wp = wavPath {
            feedback.updateWavPath(wp)
        }
        if let open = isWheelOpen {
            wheelOpen = open
        }
    }

    func run(dragKey: String, idleShortcutSpec: String, mouseButton: Int?, wavPath: String?) -> Never {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)

        configure(dragKey: dragKey, idleShortcutSpec: idleShortcutSpec, mouseButton: mouseButton, wavPath: wavPath, isWheelOpen: false)
        lastChangeCount = dragPasteboard.changeCount

        let axTrusted = AXIsProcessTrusted()
        setupInputMonitors(axTrusted: axTrusted)
        setupDragPolling()
        setupStdinReader()

        writeJSON([
            "event": "ready",
            "axTrusted": axTrusted,
            "dragKeyCode": Int(dragKeyCode)
        ])

        app.run()
        exit(0)
    }

    private func setupStdinReader() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            while let line = readLine() {
                let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.isEmpty { continue }
                guard let data = trimmed.data(using: .utf8),
                      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let cmd = json["cmd"] as? String else {
                    continue
                }
                DispatchQueue.main.async {
                    guard let self = self else { return }
                    switch cmd {
                    case "set_config":
                        let mb: Int?
                        if json.keys.contains("mouseButton") {
                            mb = (json["mouseButton"] as? Int) ?? -1
                        } else {
                            mb = nil
                        }
                        self.configure(
                            dragKey: json["dragKey"] as? String,
                            idleShortcutSpec: json["idleShortcut"] as? String,
                            mouseButton: mb,
                            wavPath: json["wavPath"] as? String,
                            isWheelOpen: json["wheelOpen"] as? Bool
                        )
                    case "wheel_state":
                        self.wheelOpen = (json["open"] as? Bool) ?? false
                    case "feedback":
                        if let kind = json["kind"] as? String {
                            self.feedback.play(kind: kind)
                        }
                    case "recheck_permissions":
                        self.refreshAccessibilityMonitor(emitChange: true)
                    case "quit":
                        exit(0)
                    default:
                        break
                    }
                }
            }
            // Parent closed stdin -> exit monitor cleanly
            exit(0)
        }
    }

    private func setupDragPolling() {
        // Lightweight timer + event-driven drag detection.
        // When no mouse button is pressed and drag is inactive, checks changeCount in O(1).
        dragTimer = Timer.scheduledTimer(withTimeInterval: 0.03, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            self.checkDragState()
            self.axPollTick += 1
            if self.axPollTick >= 50 {
                self.axPollTick = 0
                self.refreshAccessibilityMonitor(emitChange: true)
            }
        }
        RunLoop.main.add(dragTimer!, forMode: .common)

        NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDragged, .leftMouseUp, .mouseMoved]) { [weak self] event in
            guard let self = self else { return }
            if event.type == .leftMouseUp {
                self.checkDragState(forceEndIfUp: true)
            } else if event.type == .leftMouseDragged {
                self.checkDragState()
                if self.wheelOpen || self.mouseTriggerHeld {
                    let pos = currentCursorAndScreen()
                    writeJSON(["event": "mouse_move", "x": pos.x, "y": pos.y])
                }
            } else if event.type == .mouseMoved && (self.wheelOpen || self.mouseTriggerHeld) {
                let pos = currentCursorAndScreen()
                writeJSON(["event": "mouse_move", "x": pos.x, "y": pos.y])
            }
        }
    }

    private func readDragFileURLs() -> [String] {
        var candidateURLs: [URL] = []
        let options: [NSPasteboard.ReadingOptionKey: Any] = [
            .urlReadingFileURLsOnly: true
        ]
        if let urls = dragPasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL], !urls.isEmpty {
            candidateURLs = urls
        } else if let names = dragPasteboard.propertyList(forType: NSPasteboard.PasteboardType("NSFilenamesPboardType")) as? [String], !names.isEmpty {
            candidateURLs = names.map { URL(fileURLWithPath: $0) }
        }

        if candidateURLs.isEmpty || candidateURLs.count > 128 {
            return []
        }

        var seen = Set<String>()
        var result: [String] = []
        result.reserveCapacity(candidateURLs.count)

        for url in candidateURLs {
            guard url.isFileURL else { continue }
            let path = url.path
            guard !path.isEmpty, FileManager.default.fileExists(atPath: path) else { continue }

            let rv = try? url.resourceValues(forKeys: [
                .isDirectoryKey,
                .isPackageKey,
                .isSymbolicLinkKey,
                .volumeIsLocalKey
            ])
            if rv?.isSymbolicLink == true {
                continue
            }
            if rv?.volumeIsLocal == false {
                continue
            }
            let isDir = rv?.isDirectory ?? false
            let isPkg = (rv?.isPackage ?? false) || NSWorkspace.shared.isFilePackage(atPath: path)
            let ext = url.pathExtension.lowercased()
            if isDir && !(isPkg && ["pages", "numbers", "key"].contains(ext)) {
                continue
            }
            let stdPath = (path as NSString).standardizingPath
            if seen.insert(stdPath).inserted {
                result.append(stdPath)
            }
            if result.count > 128 {
                return []
            }
        }
        return result
    }

    private func checkDragState(forceEndIfUp: Bool = false) {
        let leftDown = (NSEvent.pressedMouseButtons & 1) != 0
        let count = dragPasteboard.changeCount

        if count != lastChangeCount {
            lastChangeCount = count
            let files = readDragFileURLs()
            if !files.isEmpty && leftDown && !forceEndIfUp {
                let changed = !dragActive || files != dragFiles
                dragActive = true
                dragFiles = files
                if changed {
                    let pos = currentCursorAndScreen()
                    writeJSON([
                        "event": "drag_start",
                        "files": files,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                    if dragKeyHeld {
                        dragKeyConsumed = true
                        writeJSON([
                            "event": "trigger_drag_wheel",
                            "files": files,
                            "x": pos.x,
                            "y": pos.y,
                            "screen": pos.screen.toDict()
                        ])
                    }
                }
            }
        } else if !dragActive && leftDown && !forceEndIfUp {
            // In case drag began before monitor attached or changeCount wasn't observed
            let files = readDragFileURLs()
            if !files.isEmpty {
                dragActive = true
                dragFiles = files
                let pos = currentCursorAndScreen()
                writeJSON([
                    "event": "drag_start",
                    "files": files,
                    "x": pos.x,
                    "y": pos.y,
                    "screen": pos.screen.toDict()
                ])
                if dragKeyHeld {
                    dragKeyConsumed = true
                    writeJSON([
                        "event": "trigger_drag_wheel",
                        "files": files,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                }
            }
        }

        if dragActive && (!leftDown || forceEndIfUp) {
            dragActive = false
            let pos = currentCursorAndScreen()
            writeJSON([
                "event": "drag_end",
                "x": pos.x,
                "y": pos.y
            ])
            dragFiles = []
        }
    }

    private func tearDownMonitors() {
        if let source = runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes)
            runLoopSource = nil
        }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
            eventTap = nil
        }
        if let fm = fallbackMonitor {
            NSEvent.removeMonitor(fm)
            fallbackMonitor = nil
        }
    }

    private func refreshAccessibilityMonitor(emitChange: Bool) {
        let nowTrusted = AXIsProcessTrusted()
        if nowTrusted == currentAxTrusted {
            return
        }
        setupInputMonitors(axTrusted: nowTrusted)
        if emitChange {
            writeJSON([
                "event": "ax_status",
                "axTrusted": currentAxTrusted
            ])
        }
    }

    private func setupInputMonitors(axTrusted: Bool) {
        tearDownMonitors()
        if axTrusted {
            let mask = (1 << CGEventType.keyDown.rawValue)
                | (1 << CGEventType.keyUp.rawValue)
                | (1 << CGEventType.flagsChanged.rawValue)
                | (1 << CGEventType.otherMouseDown.rawValue)
                | (1 << CGEventType.otherMouseUp.rawValue)

            let callback: CGEventTapCallBack = { _, type, event, _ in
                return MonitorController.shared.handleCGEvent(type: type, event: event)
            }

            if let tap = CGEvent.tapCreate(
                tap: .cgSessionEventTap,
                place: .headInsertEventTap,
                options: .defaultTap,
                eventsOfInterest: CGEventMask(mask),
                callback: callback,
                userInfo: nil
            ) {
                self.eventTap = tap
                let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
                self.runLoopSource = source
                CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
                CGEvent.tapEnable(tap: tap, enable: true)
                self.currentAxTrusted = true
                return
            }
        }

        self.currentAxTrusted = false
        // Zero-permission fallback monitor (observes events without consuming drag key)
        self.fallbackMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged, .otherMouseDown, .otherMouseUp]) { [weak self] event in
            self?.handleFallbackNSEvent(event)
        }
    }

    private func handleCGEvent(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap = eventTap {
                CGEvent.tapEnable(tap: tap, enable: true)
            }
            return Unmanaged.passUnretained(event)
        }

        if type == .otherMouseDown || type == .otherMouseUp {
            let btn = event.getIntegerValueField(.mouseEventButtonNumber)
            if let targetBtn = mouseTriggerButton, btn == targetBtn {
                let pos = currentCursorAndScreen()
                if type == .otherMouseDown {
                    mouseTriggerHeld = true
                    writeJSON([
                        "event": "trigger_mouse_wheel",
                        "button": Int(btn),
                        "pressed": true,
                        "files": dragActive ? dragFiles : [],
                        "dragging": dragActive,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                } else {
                    mouseTriggerHeld = false
                    writeJSON([
                        "event": "mouse_release",
                        "button": Int(btn),
                        "x": pos.x,
                        "y": pos.y
                    ])
                }
                return nil // Consume configured trigger mouse button
            }
            return Unmanaged.passUnretained(event)
        }

        let keyCode = UInt16(event.getIntegerValueField(.keyboardEventKeycode))
        let flags = event.flags
        let isRepeat = event.getIntegerValueField(.keyboardEventAutorepeat) != 0

        if type == .keyDown && keyCode == 53 && wheelOpen {
            writeJSON(["event": "escape"])
            return nil
        }

        // 1) Idle Global Shortcut
        if type == .keyDown && !isRepeat && !dragActive, let idle = idleShortcut, idle.matches(keyCode: keyCode, flags: flags) {
            let pos = currentCursorAndScreen()
            writeJSON([
                "event": "trigger_idle_wheel",
                "x": pos.x,
                "y": pos.y,
                "screen": pos.screen.toDict()
            ])
            return nil
        }

        // 2) Drag Trigger Key (consumed ONLY when dragActive or already consumed for this press)
        if dragModifierOnly == nil && keyCode == dragKeyCode {
            let disallowedMods: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl, .maskShift]
            let hasDisallowedMod = !flags.intersection(disallowedMods).isEmpty
            if type == .keyDown {
                if !isRepeat {
                    if hasDisallowedMod {
                        dragKeyHeld = false
                        dragKeyConsumed = false
                        return Unmanaged.passUnretained(event)
                    }
                    dragKeyHeld = true
                    if dragActive {
                        dragKeyConsumed = true
                        let pos = currentCursorAndScreen()
                        writeJSON([
                            "event": "trigger_drag_wheel",
                            "files": dragFiles,
                            "x": pos.x,
                            "y": pos.y,
                            "screen": pos.screen.toDict()
                        ])
                        return nil
                    }
                    dragKeyConsumed = false
                } else if dragKeyConsumed {
                    return nil
                }
            } else if type == .keyUp {
                dragKeyHeld = false
                if dragKeyConsumed {
                    dragKeyConsumed = false
                    return nil
                }
            }
        } else if type == .flagsChanged, let mod = dragModifierOnly {
            let pressed = modifierActive(mod, flags: flags)
            if pressed && !dragKeyHeld {
                dragKeyHeld = true
                if dragActive {
                    let pos = currentCursorAndScreen()
                    writeJSON([
                        "event": "trigger_drag_wheel",
                        "files": dragFiles,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                }
            } else if !pressed {
                dragKeyHeld = false
            }
        }

        return Unmanaged.passUnretained(event)
    }

    private func handleFallbackNSEvent(_ event: NSEvent) {
        if event.type == .otherMouseDown || event.type == .otherMouseUp {
            let btn = Int64(event.buttonNumber)
            if let targetBtn = mouseTriggerButton, btn == targetBtn {
                let pos = currentCursorAndScreen()
                if event.type == .otherMouseDown {
                    mouseTriggerHeld = true
                    writeJSON([
                        "event": "trigger_mouse_wheel",
                        "button": Int(btn),
                        "pressed": true,
                        "files": dragActive ? dragFiles : [],
                        "dragging": dragActive,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                } else {
                    mouseTriggerHeld = false
                    writeJSON([
                        "event": "mouse_release",
                        "button": Int(btn),
                        "x": pos.x,
                        "y": pos.y
                    ])
                }
            }
            return
        }

        if event.type == .keyDown && event.keyCode == 53 && wheelOpen {
            writeJSON(["event": "escape"])
            return
        }

        if event.type == .keyDown && !event.isARepeat && !dragActive, let idle = idleShortcut, idle.matches(event: event) {
            let pos = currentCursorAndScreen()
            writeJSON([
                "event": "trigger_idle_wheel",
                "x": pos.x,
                "y": pos.y,
                "screen": pos.screen.toDict()
            ])
            return
        }

        if dragModifierOnly == nil && event.keyCode == dragKeyCode {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let hasDisallowedMod = !modFlags.intersection([.command, .option, .control, .shift]).isEmpty
            if event.type == .keyDown && !event.isARepeat {
                if hasDisallowedMod {
                    dragKeyHeld = false
                    return
                }
                dragKeyHeld = true
                if dragActive {
                    let pos = currentCursorAndScreen()
                    writeJSON([
                        "event": "trigger_drag_wheel",
                        "files": dragFiles,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                }
            } else if event.type == .keyUp {
                dragKeyHeld = false
            }
        } else if event.type == .flagsChanged, let mod = dragModifierOnly {
            let pressed = modifierActive(mod, nsFlags: event.modifierFlags)
            if pressed && !dragKeyHeld {
                dragKeyHeld = true
                if dragActive {
                    let pos = currentCursorAndScreen()
                    writeJSON([
                        "event": "trigger_drag_wheel",
                        "files": dragFiles,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                }
            } else if !pressed {
                dragKeyHeld = false
            }
        }
    }

    private func modifierActive(_ mod: String, flags: CGEventFlags) -> Bool {
        switch mod {
        case "option", "alt": return flags.contains(.maskAlternate)
        case "control", "ctrl": return flags.contains(.maskControl)
        case "shift": return flags.contains(.maskShift)
        case "command", "cmd": return flags.contains(.maskCommand)
        case "fn": return flags.contains(.maskSecondaryFn)
        default: return false
        }
    }

    private func modifierActive(_ mod: String, nsFlags: NSEvent.ModifierFlags) -> Bool {
        switch mod {
        case "option", "alt": return nsFlags.contains(.option)
        case "control", "ctrl": return nsFlags.contains(.control)
        case "shift": return nsFlags.contains(.shift)
        case "command", "cmd": return nsFlags.contains(.command)
        case "fn": return nsFlags.contains(.function)
        default: return false
        }
    }
}
