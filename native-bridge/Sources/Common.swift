import Foundation
import AppKit
import CoreGraphics

enum BridgeError: Error, LocalizedError {
    case invalidArguments(String)
    case operationFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidArguments(let msg): return msg
        case .operationFailed(let msg): return msg
        }
    }
}

func writeJSON(_ object: Any) {
    do {
        let data = try JSONSerialization.data(withJSONObject: object, options: [])
        if let str = String(data: data, encoding: .utf8) {
            FileHandle.standardOutput.write(Data((str + "\n").utf8))
            fflush(stdout)
        }
    } catch {
        let fallback = "{\"event\":\"error\",\"message\":\"JSON serialization failed\"}\n"
        FileHandle.standardOutput.write(Data(fallback.utf8))
        fflush(stdout)
    }
}

func failExit(_ message: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

struct ScreenGeometry {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
    let visibleX: Double
    let visibleY: Double
    let visibleWidth: Double
    let visibleHeight: Double
    let scale: Double

    func toDict() -> [String: Any] {
        [
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "visibleX": visibleX,
            "visibleY": visibleY,
            "visibleWidth": visibleWidth,
            "visibleHeight": visibleHeight,
            "scale": scale
        ]
    }
}

func currentCursorAndScreen() -> (x: Double, y: Double, screen: ScreenGeometry) {
    let cocoaPoint = NSEvent.mouseLocation
    let screens = NSScreen.screens
    let primaryHeight = Double(screens.first?.frame.height ?? 1080)
    let activeScreen = screens.first { NSMouseInRect(cocoaPoint, $0.frame, false) } ?? NSScreen.main ?? screens.first

    let topLeftX = Double(cocoaPoint.x)
    let topLeftY = primaryHeight - Double(cocoaPoint.y)

    if let s = activeScreen {
        let f = s.frame
        let vf = s.visibleFrame
        let geom = ScreenGeometry(
            x: Double(f.minX),
            y: primaryHeight - Double(f.maxY),
            width: Double(f.width),
            height: Double(f.height),
            visibleX: Double(vf.minX),
            visibleY: primaryHeight - Double(vf.maxY),
            visibleWidth: Double(vf.width),
            visibleHeight: Double(vf.height),
            scale: Double(s.backingScaleFactor)
        )
        return (topLeftX, topLeftY, geom)
    }

    let fallback = ScreenGeometry(
        x: 0, y: 0, width: 1440, height: 900,
        visibleX: 0, visibleY: 25, visibleWidth: 1440, visibleHeight: 875, scale: 2.0
    )
    return (topLeftX, topLeftY, fallback)
}

func keyCodeForName(_ raw: String) -> UInt16? {
    let normalized = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    switch normalized {
    case "down", "downarrow", "arrowdown", "down arrow": return 125
    case "up", "uparrow", "arrowup", "up arrow": return 126
    case "left", "leftarrow", "arrowleft", "left arrow": return 123
    case "right", "rightarrow", "arrowright", "right arrow": return 124
    case "space", "spacebar": return 49
    case "tab": return 48
    case "return", "enter": return 36
    case "escape", "esc": return 53
    case "f1": return 122
    case "f2": return 120
    case "f3": return 99
    case "f4": return 118
    case "f5": return 96
    case "f6": return 97
    case "f7": return 98
    case "f8": return 100
    case "f9": return 101
    case "f10": return 109
    case "f11": return 103
    case "f12": return 111
    case "fn", "function", "globe": return 63
    case "a": return 0
    case "s": return 1
    case "d": return 2
    case "f": return 3
    case "h": return 4
    case "g": return 5
    case "z": return 6
    case "x": return 7
    case "c": return 8
    case "v": return 9
    case "b": return 11
    case "q": return 12
    case "w": return 13
    case "e": return 14
    case "r": return 15
    case "y": return 16
    case "t": return 17
    case "o": return 31
    case "u": return 32
    case "i": return 34
    case "p": return 35
    case "l": return 37
    case "j": return 38
    case "k": return 40
    case "n": return 45
    case "m": return 46
    case "1": return 18
    case "2": return 19
    case "3": return 20
    case "4": return 21
    case "5": return 23
    case "6": return 22
    case "7": return 26
    case "8": return 28
    case "9": return 25
    case "0": return 29
    default: return nil
    }
}

struct ParsedShortcut {
    let keyCode: UInt16
    let command: Bool
    let option: Bool
    let control: Bool
    let shift: Bool

    static func parse(_ spec: String) -> ParsedShortcut? {
        let tokens = spec
            .replacingOccurrences(of: "⌘", with: "Command+")
            .replacingOccurrences(of: "⌥", with: "Option+")
            .replacingOccurrences(of: "⌃", with: "Control+")
            .replacingOccurrences(of: "⇧", with: "Shift+")
            .split(separator: "+")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }

        guard let keyToken = tokens.last, let code = keyCodeForName(keyToken) else {
            return nil
        }

        var cmd = false, opt = false, ctrl = false, shf = false
        for mod in tokens.dropLast() {
            switch mod {
            case "cmd", "command", "super", "meta": cmd = true
            case "opt", "option", "alt": opt = true
            case "ctrl", "control": ctrl = true
            case "shift": shf = true
            default: return nil
            }
        }
        return ParsedShortcut(keyCode: code, command: cmd, option: opt, control: ctrl, shift: shf)
    }

    func matches(keyCode: UInt16, flags: CGEventFlags) -> Bool {
        guard self.keyCode == keyCode else { return false }
        let hasCmd = flags.contains(.maskCommand)
        let hasOpt = flags.contains(.maskAlternate)
        let hasCtrl = flags.contains(.maskControl)
        let hasShift = flags.contains(.maskShift)
        return hasCmd == command && hasOpt == option && hasCtrl == control && hasShift == shift
    }

    func matches(event: NSEvent) -> Bool {
        guard self.keyCode == event.keyCode else { return false }
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        return flags.contains(.command) == command
            && flags.contains(.option) == option
            && flags.contains(.control) == control
            && flags.contains(.shift) == shift
    }
}
