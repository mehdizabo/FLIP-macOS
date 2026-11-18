import Foundation
import AppKit
import CoreGraphics
import ApplicationServices

enum FinderBridge {
    private static let appleScriptQueue = DispatchQueue(label: "com.thetinyfoundry.flip.applescript")

    static func runAppleScriptDetailed(_ source: String, timeoutSeconds: TimeInterval = 5.0) -> (descriptor: NSAppleEventDescriptor?, error: String?, errorNumber: Int?) {
        let sem = DispatchSemaphore(value: 0)
        var outDesc: NSAppleEventDescriptor?
        var outErr: String?
        var outErrNum: Int?

        appleScriptQueue.async {
            guard let script = NSAppleScript(source: source) else {
                outErr = "Failed to compile AppleScript"
                sem.signal()
                return
            }
            var errInfo: NSDictionary?
            let result = script.executeAndReturnError(&errInfo)
            if let errInfo = errInfo {
                outErr = (errInfo[NSAppleScript.errorMessage] as? String) ?? "AppleScript error"
                outErrNum = (errInfo[NSAppleScript.errorNumber] as? NSNumber)?.intValue
            } else {
                outDesc = result
            }
            sem.signal()
        }

        if sem.wait(timeout: .now() + timeoutSeconds) == .timedOut {
            return (nil, "AppleScript timed out after \(Int(timeoutSeconds))s", nil)
        }
        return (outDesc, outErr, outErrNum)
    }

    static func runAppleScript(_ source: String, timeoutSeconds: TimeInterval = 5.0) -> (descriptor: NSAppleEventDescriptor?, error: String?) {
        let res = runAppleScriptDetailed(source, timeoutSeconds: timeoutSeconds)
        return (res.descriptor, res.error)
    }

    private static func isAutomationDenied(error: String, errorNumber: Int?) -> Bool {
        if let code = errorNumber, code == -1743 || code == -1744 {
            return true
        }
        if error.localizedCaseInsensitiveContains("Not authorized") {
            return true
        }
        let finderTarget = NSAppleEventDescriptor(bundleIdentifier: "com.apple.finder")
        if let aeDesc = finderTarget.aeDesc {
            let status = AEDeterminePermissionToAutomateTarget(aeDesc, typeWildCard, typeWildCard, false)
            return status == OSStatus(errAEEventNotPermitted) || status == OSStatus(errAEEventWouldRequireUserConsent)
        }
        return false
    }

    private static func isTrashFolder(_ path: String) -> Bool {
        let standardized = URL(fileURLWithPath: path).resolvingSymlinksInPath().standardizedFileURL.path
        let rawTrash = FileManager.default.urls(for: .trashDirectory, in: .userDomainMask).first?.path
            ?? (NSHomeDirectory() + "/.Trash")
        let trashPath = URL(fileURLWithPath: rawTrash).resolvingSymlinksInPath().standardizedFileURL.path
        if standardized == trashPath || standardized.hasPrefix(trashPath + "/") {
            return true
        }
        return standardized == "/.Trashes"
            || standardized.hasPrefix("/.Trashes/")
            || standardized.contains("/.Trashes/")
    }

    static func getFinderContext() -> [String: Any] {
        let frontmostId = NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? ""

        let isFinderRunning = !NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder").isEmpty
        if !isFinderRunning {
            return [
                "frontmostBundleId": frontmostId,
                "selectedFiles": [],
                "targetFolder": "",
                "automationGranted": false,
                "error": "Finder is not running"
            ]
        }

        let script = """
        with timeout of 3 seconds
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
        end timeout
        """

        let (desc, err, errNum) = runAppleScriptDetailed(script, timeoutSeconds: 4.0)
        if let err = err {
            return [
                "frontmostBundleId": frontmostId,
                "selectedFiles": [],
                "targetFolder": "",
                "automationGranted": !isAutomationDenied(error: err, errorNumber: errNum),
                "error": err
            ]
        }

        var targetFolder = ""
        var selectedFiles: [String] = []

        if let desc = desc, desc.numberOfItems >= 2 {
            if let folderStr = desc.atIndex(1)?.stringValue, !folderStr.isEmpty {
                let normalized = (folderStr as NSString).standardizingPath
                var isDir: ObjCBool = false
                if FileManager.default.fileExists(atPath: normalized, isDirectory: &isDir),
                   isDir.boolValue,
                   !isTrashFolder(normalized),
                   !NSWorkspace.shared.isFilePackage(atPath: normalized) {
                    targetFolder = normalized
                }
            }
            if let listDesc = desc.atIndex(2), listDesc.numberOfItems > 0 {
                for i in 1...listDesc.numberOfItems {
                    if let rawPath = listDesc.atIndex(i)?.stringValue, !rawPath.isEmpty {
                        let cleaned = rawPath.hasSuffix("/") && rawPath.count > 1 ? String(rawPath.dropLast()) : rawPath
                        var isDir: ObjCBool = false
                        let exists = FileManager.default.fileExists(atPath: cleaned, isDirectory: &isDir)
                        let ext = URL(fileURLWithPath: cleaned).pathExtension.lowercased()
                        let isIWorkPackage = exists && NSWorkspace.shared.isFilePackage(atPath: cleaned) && ["pages", "numbers", "key"].contains(ext)
                        if exists && (!isDir.boolValue || isIWorkPackage) {
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

    /// Queries raw Finder selection including plain directories so `revealFiles(..., rename: true)`
    /// can confirm selection of newly created folders (`untitled folder`) as well as files.
    private static func rawFinderSelection() -> (items: [String], automationGranted: Bool) {
        let script = """
        with timeout of 2 seconds
            tell application "Finder"
                set selPaths to {}
                try
                    set selList to selection as alias list
                    repeat with itemRef in selList
                        set end of selPaths to POSIX path of itemRef
                    end repeat
                on error
                    try
                        set singleItem to (selection as alias)
                        set end of selPaths to POSIX path of singleItem
                    end try
                end try
                return selPaths
            end tell
        end timeout
        """
        let (desc, err, errNum) = runAppleScriptDetailed(script, timeoutSeconds: 2.5)
        if let err = err {
            return ([], !isAutomationDenied(error: err, errorNumber: errNum))
        }
        var items: [String] = []
        if let desc = desc, desc.numberOfItems > 0 {
            for i in 1...desc.numberOfItems {
                if let rawPath = desc.atIndex(i)?.stringValue, !rawPath.isEmpty {
                    let cleaned = rawPath.hasSuffix("/") && rawPath.count > 1 ? String(rawPath.dropLast()) : rawPath
                    items.append(cleaned)
                }
            }
        }
        return (items, true)
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
                    let targetStd = urls[0].resolvingSymlinksInPath().standardizedFileURL.path
                    var selectionConfirmed = false
                    var automationBlocked = false
                    for _ in 0..<10 {
                        Thread.sleep(forTimeInterval: 0.04)
                        let (sel, granted) = rawFinderSelection()
                        if !granted {
                            automationBlocked = true
                            break
                        }
                        if sel.contains(where: { URL(fileURLWithPath: $0).resolvingSymlinksInPath().standardizedFileURL.path == targetStd }) {
                            selectionConfirmed = true
                            break
                        }
                    }
                    if automationBlocked {
                        Thread.sleep(forTimeInterval: 0.14)
                    }

                    if selectionConfirmed || automationBlocked {
                        let pid = finderApp.processIdentifier
                        if let src = CGEventSource(stateID: .privateState) ?? CGEventSource(stateID: .hidSystemState),
                           let keyDown = CGEvent(keyboardEventSource: src, virtualKey: 36, keyDown: true),
                           let keyUp = CGEvent(keyboardEventSource: src, virtualKey: 36, keyDown: false) {
                            keyDown.flags = []
                            keyUp.flags = []
                            keyDown.postToPid(pid)
                            keyUp.postToPid(pid)
                            renameTriggered = true
                        } else {
                            renameError = "Could not create CGEvent for Return key"
                        }
                    } else {
                        renameError = "Skipped inline rename because Finder selection did not match created item"
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
        if FileManager.default.fileExists(atPath: targetPath)
            || (try? FileManager.default.attributesOfItem(atPath: targetPath)) != nil {
            return ["ok": false, "error": "Target path already exists: \(targetPath)"]
        }
        let pb = NSPasteboard.general
        let types = (pb.types ?? []).map { $0.rawValue }
        let outURL = URL(fileURLWithPath: targetPath)

        if clipboardHasTrueImage(pb, types: types) {
            if let pngData = pb.data(forType: .png) {
                do {
                    try pngData.write(to: outURL, options: .withoutOverwriting)
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
                    try pngData.write(to: outURL, options: .withoutOverwriting)
                    return ["ok": true, "kind": "image", "path": targetPath]
                } catch {
                    return ["ok": false, "error": error.localizedDescription]
                }
            }
        }
        if let pdfData = pb.data(forType: .pdf) {
            do {
                try pdfData.write(to: outURL, options: .withoutOverwriting)
                return ["ok": true, "kind": "pdf", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let rtfData = pb.data(forType: .rtf) {
            do {
                try rtfData.write(to: outURL, options: .withoutOverwriting)
                return ["ok": true, "kind": "rtf", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        if let urlStr = clipboardWebURL(pb) {
            let plain = pb.string(forType: .string)?.trimmingCharacters(in: .whitespacesAndNewlines)
            if (plain == nil || plain == urlStr), let plistData = weblocData(for: urlStr) {
                do {
                    try plistData.write(to: outURL, options: .withoutOverwriting)
                    return ["ok": true, "kind": "webloc", "path": targetPath]
                } catch {
                    return ["ok": false, "error": error.localizedDescription]
                }
            }
        }
        if let str = pb.string(forType: .string), !str.isEmpty {
            do {
                try Data(str.utf8).write(to: outURL, options: .withoutOverwriting)
                return ["ok": true, "kind": "text", "path": targetPath]
            } catch {
                return ["ok": false, "error": error.localizedDescription]
            }
        }
        return ["ok": false, "error": "Clipboard does not contain supported image, PDF, RTF, URL, or text data"]
    }

    static func checkPermissions(
        promptAx: Bool,
        openAx: Bool,
        openAutomation: Bool,
        openInputMonitoring: Bool = false
    ) -> [String: Any] {
        let options = ["AXTrustedCheckOptionPrompt": promptAx] as CFDictionary
        let axTrusted = AXIsProcessTrustedWithOptions(options)
        let inputMonitoring = CGPreflightListenEventAccess()

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
        if openInputMonitoring, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent") {
            NSWorkspace.shared.open(url)
        }

        return [
            "accessibility": axTrusted,
            "automationFinder": automationStatus,
            "inputMonitoring": inputMonitoring
        ]
    }

    static func sendNotification(title: String, body: String, revealPath: String?) -> [String: Any] {
        let cleanTitle = String(title.trimmingCharacters(in: .whitespacesAndNewlines).prefix(160))
        let cleanBody = String(body.trimmingCharacters(in: .whitespacesAndNewlines).prefix(320))
        guard !cleanTitle.isEmpty, !cleanBody.isEmpty else {
            return [
                "ok": false,
                "method": "osascript",
                "error": "Notification title and body must be non-empty"
            ]
        }

        let trimmedReveal = revealPath?.trimmingCharacters(in: .whitespacesAndNewlines)
        let effectiveReveal = (trimmedReveal?.isEmpty == false) ? trimmedReveal : nil
        let subtitle: String = {
            guard let rp = effectiveReveal else { return "" }
            let url = URL(fileURLWithPath: rp)
            let folder = url.deletingLastPathComponent().lastPathComponent
            return folder.isEmpty ? url.lastPathComponent : "Saved in \(folder)"
        }()

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        if subtitle.isEmpty {
            proc.arguments = [
                "-e", "on run argv",
                "-e", "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e", "end run",
                "--", cleanTitle, cleanBody
            ]
        } else {
            proc.arguments = [
                "-e", "on run argv",
                "-e", "display notification (item 2 of argv) with title (item 1 of argv) subtitle (item 3 of argv)",
                "-e", "end run",
                "--", cleanTitle, cleanBody, subtitle
            ]
        }

        let errPipe = Pipe()
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = errPipe

        let sem = DispatchSemaphore(value: 0)
        proc.terminationHandler = { _ in
            sem.signal()
        }

        do {
            try proc.run()
        } catch {
            return [
                "ok": false,
                "method": "osascript",
                "error": error.localizedDescription
            ]
        }

        if sem.wait(timeout: .now() + 5.0) == .timedOut {
            proc.terminate()
            return [
                "ok": false,
                "method": "osascript",
                "error": "osascript notification timed out"
            ]
        }

        let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
        let errStr = String(data: errData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

        if proc.terminationStatus != 0 {
            return [
                "ok": false,
                "method": "osascript",
                "error": errStr.isEmpty ? "osascript exited with status \(proc.terminationStatus)" : errStr
            ]
        }

        return [
            "ok": true,
            "method": "osascript",
            "title": cleanTitle,
            "body": cleanBody,
            "revealPath": effectiveReveal as Any? ?? NSNull()
        ]
    }
}
