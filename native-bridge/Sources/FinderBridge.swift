import Foundation
import AppKit
import ApplicationServices

enum FinderBridge {
    static func runAppleScript(_ source: String) -> (descriptor: NSAppleEventDescriptor?, error: String?) {
        guard let script = NSAppleScript(source: source) else {
            return (nil, "Failed to compile AppleScript")
        }
        var errInfo: NSDictionary?
        let result = script.executeAndReturnError(&errInfo)
        if let errInfo = errInfo {
            let msg = (errInfo[NSAppleScript.errorMessage] as? String) ?? "AppleScript error"
            return (nil, msg)
        }
        return (result, nil)
    }

    static func getFinderContext() -> [String: Any] {
        let frontmostId = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? ""
        let desktopPath = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first?.path
            ?? (NSHomeDirectory() + "/Desktop")

        let isFinderRunning = !NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder").isEmpty
        if !isFinderRunning {
            return [
                "frontmostBundleId": frontmostId,
                "selectedFiles": [],
                "targetFolder": desktopPath,
                "automationGranted": false,
                "error": "Finder is not running"
            ]
        }

        let script = """
        tell application "Finder"
            set selPaths to {}
            try
                set selList to selection as alias list
                repeat with itemRef in selList
                    set itemPath to POSIX path of itemRef
                    set end of selPaths to itemPath
                end repeat
            on error
                try
                    set singleItem to (selection as alias)
                    set end of selPaths to POSIX path of singleItem
                end try
            end try
            set targetPath to ""
            try
                set insLoc to (insertion location as alias)
                set targetPath to POSIX path of insLoc
            end try
            return {targetPath, selPaths}
        end tell
        """

        let (desc, err) = runAppleScript(script)
        if let err = err {
            return [
                "frontmostBundleId": frontmostId,
                "selectedFiles": [],
                "targetFolder": desktopPath,
                "automationGranted": !err.localizedCaseInsensitiveContains("Not authorized"),
                "error": err
            ]
        }

        var targetFolder = desktopPath
        var selectedFiles: [String] = []

        if let desc = desc, desc.numberOfItems >= 2 {
            if let folderStr = desc.atIndex(1)?.stringValue, !folderStr.isEmpty {
                let normalized = (folderStr as NSString).standardizingPath
                var isDir: ObjCBool = false
                if FileManager.default.fileExists(atPath: normalized, isDirectory: &isDir), isDir.boolValue {
                    targetFolder = normalized
                }
            }
            if let listDesc = desc.atIndex(2), listDesc.numberOfItems > 0 {
                for i in 1...listDesc.numberOfItems {
                    if let rawPath = listDesc.atIndex(i)?.stringValue, !rawPath.isEmpty {
                        let cleaned = rawPath.hasSuffix("/") && rawPath.count > 1 ? String(rawPath.dropLast()) : rawPath
                        var isDir: ObjCBool = false
                        if FileManager.default.fileExists(atPath: cleaned, isDirectory: &isDir), !isDir.boolValue {
                            selectedFiles.append(cleaned)
                        }
                    }
                }
            }
        }

        return [
            "frontmostBundleId": frontmostId,
            "selectedFiles": selectedFiles,
            "targetFolder": targetFolder,
            "automationGranted": true,
            "error": NSNull()
        ]
    }

    static func revealFiles(_ paths: [String], rename: Bool) -> [String: Any] {
        let urls = paths.map { URL(fileURLWithPath: $0) }
        guard !urls.isEmpty else {
            return ["ok": false, "renameTriggered": false, "error": "No paths supplied"]
        }

        NSWorkspace.shared.activateFileViewerSelecting(urls)

        var renameTriggered = false
        var renameError: String? = nil

        if rename && urls.count == 1 {
            if AXIsProcessTrusted() {
                if let finderApp = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder").first {
                    finderApp.activate(options: [])
                    Thread.sleep(forTimeInterval: 0.16)
                    let pid = finderApp.processIdentifier
                    if let src = CGEventSource(stateID: .hidSystemState),
                       let keyDown = CGEvent(keyboardEventSource: src, virtualKey: 36, keyDown: true),
                       let keyUp = CGEvent(keyboardEventSource: src, virtualKey: 36, keyDown: false) {
                        keyDown.postToPid(pid)
                        keyUp.postToPid(pid)
                        renameTriggered = true
                    } else {
                        renameError = "Could not create CGEvent for Return key"
                    }
                } else {
                    renameError = "Finder process not found"
                }
            } else {
                renameError = "Accessibility permission required for inline Finder rename"
            }
        }

        return [
            "ok": true,
            "renameTriggered": renameTriggered,
            "error": renameError as Any? ?? NSNull()
        ]
    }

    static func inspectClipboard() -> [String: Any] {
        let pb = NSPasteboard.general
        let types = (pb.types ?? []).map { $0.rawValue }
        if pb.data(forType: .png) != nil || pb.data(forType: .tiff) != nil || NSImage(pasteboard: pb) != nil {
            return ["available": true, "kind": "image", "extension": "png", "types": types]
        }
        if pb.data(forType: .pdf) != nil {
            return ["available": true, "kind": "pdf", "extension": "pdf", "types": types]
        }
        if pb.data(forType: .rtf) != nil {
            return ["available": true, "kind": "rtf", "extension": "rtf", "types": types]
        }
        if let str = pb.string(forType: .string), !str.isEmpty {
            return ["available": true, "kind": "text", "extension": "txt", "length": str.count, "types": types]
        }
        return ["available": false, "kind": "empty", "extension": "", "types": types]
    }

    static func saveClipboard(to targetPath: String) -> [String: Any] {
        let pb = NSPasteboard.general
        let outURL = URL(fileURLWithPath: targetPath)

        if let pngData = pb.data(forType: .png) {
            do {
                try pngData.write(to: outURL, options: .atomic)
                return ["ok": true, "kind": "image", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let img = NSImage(pasteboard: pb),
           let tiff = img.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiff),
           let pngData = rep.representation(using: .png, properties: [:]) {
            do {
                try pngData.write(to: outURL, options: .atomic)
                return ["ok": true, "kind": "image", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let pdfData = pb.data(forType: .pdf) {
            do {
                try pdfData.write(to: outURL, options: .atomic)
                return ["ok": true, "kind": "pdf", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let rtfData = pb.data(forType: .rtf) {
            do {
                try rtfData.write(to: outURL, options: .atomic)
                return ["ok": true, "kind": "rtf", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let str = pb.string(forType: .string), !str.isEmpty {
            do {
                try Data(str.utf8).write(to: outURL, options: .atomic)
                return ["ok": true, "kind": "text", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        return ["ok": false, "error": "Clipboard does not contain supported image, PDF, RTF, or text data"]
    }

    static func checkPermissions(promptAx: Bool, openAx: Bool, openAutomation: Bool) -> [String: Any] {
        let options = ["AXTrustedCheckOptionPrompt": promptAx] as CFDictionary
        let axTrusted = AXIsProcessTrustedWithOptions(options)

        var automationStatus = "unknown"
        let finderTarget = NSAppleEventDescriptor(bundleIdentifier: "com.apple.finder")
        if let aeDesc = finderTarget.aeDesc {
            let status = AEDeterminePermissionToAutomateTarget(aeDesc, typeWildCard, typeWildCard, false)
            switch status {
            case noErr: automationStatus = "granted"
            case OSStatus(errAEEventNotPermitted): automationStatus = "denied"
            case OSStatus(errAEEventWouldRequireUserConsent): automationStatus = "not_determined"
            case OSStatus(procNotFound): automationStatus = "finder_not_running"
            default: automationStatus = "code_\(status)"
            }
        }

        if openAx, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") {
            NSWorkspace.shared.open(url)
        }
        if openAutomation, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation") {
            NSWorkspace.shared.open(url)
        }

        return [
            "accessibility": axTrusted,
            "automationFinder": automationStatus,
            "inputMonitoring": axTrusted
        ]
    }
}
