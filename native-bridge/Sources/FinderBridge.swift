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

    private static func clipboardHasTrueImage(_ pb: NSPasteboard, types: [String]) -> Bool {
        // Finder file copies put `public.file-url` + a generic file icon TIFF on the pasteboard.
        // Only treat clipboard as an image when explicit image pasteboard data exists or there is no file-url icon shadow.
        let hasDirectImage = pb.data(forType: .png) != nil
            || types.contains("public.heic")
            || types.contains("public.jpeg")
            || types.contains("com.apple.uikit.image")
        if hasDirectImage {
            return true
        }
        if types.contains("public.file-url") {
            return false
        }
        return pb.data(forType: .tiff) != nil || NSImage(pasteboard: pb) != nil
    }

    private static func clipboardWebURL(_ pb: NSPasteboard) -> String? {
        if let urlStr = pb.string(forType: .URL)?.trimmingCharacters(in: .whitespacesAndNewlines),
           urlStr.lowercased().hasPrefix("http://") || urlStr.lowercased().hasPrefix("https://") {
            return urlStr
        }
        return nil
    }

    private static func weblocData(for urlString: String) -> Data? {
        let plist: [String: Any] = ["URL": urlString]
        return try? PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0)
    }

    static func inspectClipboard() -> [String: Any] {
        let pb = NSPasteboard.general
        let types = (pb.types ?? []).map { $0.rawValue }
        if clipboardHasTrueImage(pb, types: types) {
            return ["available": true, "kind": "image", "extension": "png", "types": types]
        }
        if pb.data(forType: .pdf) != nil {
            return ["available": true, "kind": "pdf", "extension": "pdf", "types": types]
        }
        if pb.data(forType: .rtf) != nil {
            return ["available": true, "kind": "rtf", "extension": "rtf", "types": types]
        }
        if let urlStr = clipboardWebURL(pb) {
            let plain = pb.string(forType: .string)?.trimmingCharacters(in: .whitespacesAndNewlines)
            if plain == nil || plain == urlStr {
                return ["available": true, "kind": "webloc", "extension": "webloc", "url": urlStr, "types": types]
            }
        }
        if let str = pb.string(forType: .string), !str.isEmpty {
            return ["available": true, "kind": "text", "extension": "txt", "length": str.count, "types": types]
        }
        return ["available": false, "kind": "empty", "extension": "", "types": types]
    }

    static func saveClipboard(to targetPath: String) -> [String: Any] {
        let pb = NSPasteboard.general
        let types = (pb.types ?? []).map { $0.rawValue }
        let outURL = URL(fileURLWithPath: targetPath)

        if clipboardHasTrueImage(pb, types: types) {
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
        if let urlStr = clipboardWebURL(pb) {
            let plain = pb.string(forType: .string)?.trimmingCharacters(in: .whitespacesAndNewlines)
            if (plain == nil || plain == urlStr), let plistData = weblocData(for: urlStr) {
                do {
                    try plistData.write(to: outURL, options: .atomic)
                    return ["ok": true, "kind": "webloc", "path": targetPath]
                } catch {
                    return ["ok": false, "error": error.localizedDescription]
                }
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
        return ["ok": false, "error": "Clipboard does not contain supported image, PDF, RTF, URL, or text data"]
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
