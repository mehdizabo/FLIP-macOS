import Foundation
import AppKit

func printUsage() {
    let usage = """
    flip-macos-bridge subcommands:
      monitor [--drag-key Down] [--idle-shortcut Option+Shift+Space] [--mouse-button -1] [--wav <path>] [--haptic-intensity <light|medium|firm|strong>] [--aura-enabled <true|false>]
      finder-context
      finder-reveal [--rename] <path...>
      clipboard-inspect
      clipboard-save <output-path>
      permissions [--prompt-ax] [--open-ax] [--open-automation] [--open-input-monitoring]
      thumbnail <path> [size]
      ocr <image-path> [out-txt-path]
      ocr-boxes <image-path>
      convert-pdf render <pdf> <out-dir> <format> [dpi]
      convert-pdf extract-text <pdf> <out-txt>
      convert-pdf from-images <out-pdf> <img1> [<img2>...]
      convert-pdf merge <out-pdf> <pdf1> <pdf2>...
      convert-doc <input> <format> <output> [--page-size a4|letter]
      convert-image <input> <format> <output> [quality] [preserve-exif:true|false] [strip-gps:true|false]
      remove-bg <input-image> <output-png>
      optimize-pdf <input-pdf> <output-pdf>
      pdf-printable <input-pdf> <output-pdf>
      pdfa-inspect <pdf>
      convert-av <input> <format> <output>
      create-iwork <pages|numbers|key> <output>
      feedback <tick|sound|alignment|level|generic|success|success_sound|success_haptic|error> [wav-path] [--intensity <light|medium|firm|strong>]
      notify <title> <body> [reveal-path]
    """
    print(usage)
}

/// A failure answered the way the PDF/A commands promise: one JSON line on stdout, exit status 1.
func failJSON(_ message: String) -> Never {
    writeJSON(["ok": false, "error": message])
    exit(1)
}

func runCLI() {
    let args = Array(CommandLine.arguments.dropFirst())
    guard let cmd = args.first else {
        printUsage()
        exit(0)
    }

    switch cmd {
    case "monitor":
        var dragKey = "Down"
        var idleShortcut = "Option+Shift+Space"
        var mouseButton: Int? = nil
        var wavPath: String? = nil
        var hapticIntensity: String? = nil
        var auraEnabled = true
        var i = 1
        while i < args.count {
            switch args[i] {
            case "--drag-key":
                if i + 1 < args.count { dragKey = args[i + 1]; i += 1 }
            case "--idle-shortcut":
                if i + 1 < args.count { idleShortcut = args[i + 1]; i += 1 }
            case "--mouse-button":
                if i + 1 < args.count { mouseButton = Int(args[i + 1]); i += 1 }
            case "--wav":
                if i + 1 < args.count { wavPath = args[i + 1]; i += 1 }
            case "--haptic-intensity":
                if i + 1 < args.count { hapticIntensity = args[i + 1]; i += 1 }
            case "--aura-enabled":
                if i + 1 < args.count {
                    let raw = args[i + 1].trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                    auraEnabled = !(raw == "false" || raw == "0" || raw == "no")
                    i += 1
                }
            default:
                break
            }
            i += 1
        }
        MonitorController.shared.run(dragKey: dragKey, idleShortcutSpec: idleShortcut, mouseButton: mouseButton, wavPath: wavPath, hapticIntensity: hapticIntensity, auraEnabled: auraEnabled)

    case "finder-context":
        writeJSON(FinderBridge.getFinderContext())

    case "finder-reveal":
        var rename = false
        var paths: [String] = []
        for arg in args.dropFirst() {
            if arg == "--rename" { rename = true }
            else { paths.append(arg) }
        }
        writeJSON(FinderBridge.revealFiles(paths, rename: rename))

    case "clipboard-inspect":
        writeJSON(FinderBridge.inspectClipboard())

    case "clipboard-save":
        guard args.count >= 2 else { failExit("clipboard-save requires <output-path>") }
        let res = FinderBridge.saveClipboard(to: args[1])
        writeJSON(res)
        if (res["ok"] as? Bool) != true { exit(1) }

    case "permissions":
        let promptAx = args.contains("--prompt-ax")
        let openAx = args.contains("--open-ax")
        let openAuto = args.contains("--open-automation")
        let openInput = args.contains("--open-input-monitoring")
        writeJSON(FinderBridge.checkPermissions(
            promptAx: promptAx,
            openAx: openAx,
            openAutomation: openAuto,
            openInputMonitoring: openInput
        ))

    case "thumbnail":
        guard args.count >= 2 else { failExit("thumbnail requires <path> [size]") }
        let size = args.count >= 3 ? (Double(args[2]) ?? 64.0) : 64.0
        let res = MediaAndDocs.generateThumbnail(path: args[1], size: CGFloat(size))
        writeJSON(res)
        if (res["ok"] as? Bool) != true { exit(1) }

    case "ocr":
        guard args.count >= 2 else { failExit("ocr requires <image-path> [out-txt-path]") }
        do {
            let outPath = args.count >= 3 ? args[2] : nil
            writeJSON(try MediaAndDocs.ocrImageFile(path: args[1], outTxtPath: outPath))
        } catch {
            failExit(error.localizedDescription)
        }

    case "ocr-boxes":
        guard args.count >= 2 else { failExit("ocr-boxes requires <image-path>") }
        do {
            writeJSON(try MediaAndDocs.ocrBoxes(path: args[1]))
        } catch {
            failExit(error.localizedDescription)
        }

    case "convert-pdf":
        guard args.count >= 2 else { failExit("convert-pdf requires a submode") }
        let sub = args[1]
        do {
            switch sub {
            case "render":
                guard args.count >= 5 else { failExit("convert-pdf render <pdf> <out-dir> <format> [dpi]") }
                let dpi = args.count >= 6 ? (Double(args[5]) ?? 144.0) : 144.0
                writeJSON(try MediaAndDocs.renderPDFPages(pdfPath: args[2], outDir: args[3], format: args[4], dpi: CGFloat(dpi)))
            case "extract-text":
                guard args.count >= 4 else { failExit("convert-pdf extract-text <pdf> <out-txt>") }
                writeJSON(try MediaAndDocs.extractPDFText(pdfPath: args[2], outTxtPath: args[3]))
            case "from-images":
                guard args.count >= 4 else { failExit("convert-pdf from-images <out-pdf> <img1>...") }
                writeJSON(try MediaAndDocs.imagesToPDF(outPdfPath: args[2], imagePaths: Array(args.dropFirst(3))))
            case "merge":
                guard args.count >= 4 else { failExit("convert-pdf merge <out-pdf> <pdf1> <pdf2>...") }
                writeJSON(try MediaAndDocs.mergePDFs(outPdfPath: args[2], pdfPaths: Array(args.dropFirst(3))))
            default:
                failExit("Unknown convert-pdf submode: \(sub)")
            }
        } catch {
            failExit(error.localizedDescription)
        }

    case "convert-doc":
        guard args.count >= 4 else { failExit("convert-doc <input> <format> <output> [--page-size a4|letter]") }
        var pageSize = "a4"
        if let idx = args.firstIndex(of: "--page-size"), idx + 1 < args.count {
            pageSize = args[idx + 1]
        }
        do {
            writeJSON(try MediaAndDocs.convertDocument(inputPath: args[1], format: args[2], outputPath: args[3], pageSize: pageSize))
        } catch {
            failExit(error.localizedDescription)
        }

    case "convert-image":
        guard args.count >= 4 else { failExit("convert-image <input> <format> <output> [quality] [preserve-exif] [strip-gps]") }
        let quality = args.count >= 5 ? (Double(args[4]) ?? 0.92) : 0.92
        let preserveExif = args.count >= 6 ? (args[5].lowercased() != "false" && args[5] != "0") : true
        let stripGps = args.count >= 7 ? (args[6].lowercased() == "true" || args[6] == "1") : false
        do {
            writeJSON(try MediaAndDocs.convertImage(
                inputPath: args[1],
                format: args[2],
                outputPath: args[3],
                quality: quality,
                preserveExif: preserveExif,
                stripGps: stripGps
            ))
        } catch {
            failExit(error.localizedDescription)
        }

    case "remove-bg":
        guard args.count >= 3 else { failExit("remove-bg <input-image> <output-png>") }
        do {
            writeJSON(try MediaAndDocs.removeBackground(inputPath: args[1], outputPath: args[2]))
        } catch {
            failExit(error.localizedDescription)
        }

    case "optimize-pdf":
        guard args.count >= 3 else { failExit("optimize-pdf <input-pdf> <output-pdf>") }
        do {
            writeJSON(try MediaAndDocs.optimizePDF(inputPath: args[1], outputPath: args[2]))
        } catch {
            failExit(error.localizedDescription)
        }

    case "pdf-printable":
        guard args.count >= 3 else { failJSON("pdf-printable <input-pdf> <output-pdf>") }
        let res = MediaAndDocs.makePDFPrintable(inputPath: args[1], outputPath: args[2])
        writeJSON(res)
        if (res["ok"] as? Bool) != true { exit(1) }

    case "pdfa-inspect":
        guard args.count >= 2 else { failJSON("pdfa-inspect <pdf>") }
        let res = MediaAndDocs.inspectPDFForPDFA(path: args[1])
        writeJSON(res)
        if (res["ok"] as? Bool) != true { exit(1) }

    case "create-iwork":
        guard args.count >= 3 else { failExit("create-iwork <pages|numbers|key> <output>") }
        do {
            writeJSON(try MediaAndDocs.createIWorkDocument(kind: args[1], outputPath: args[2]))
        } catch {
            failExit(error.localizedDescription)
        }

    case "convert-av":
        guard args.count >= 4 else { failExit("convert-av <input> <format> <output>") }
        do {
            writeJSON(try MediaAndDocs.convertAV(inputPath: args[1], format: args[2], outputPath: args[3]))
        } catch {
            failExit(error.localizedDescription)
        }

    case "feedback":
        guard args.count >= 2 else { failExit("feedback <kind> [wav-path] [--intensity <level>]") }
        let kind = args[1]
        var wav: String? = nil
        var intensity: String? = nil
        var i = 2
        while i < args.count {
            if args[i] == "--intensity" || args[i] == "--haptic-intensity" {
                if i + 1 < args.count {
                    intensity = args[i + 1]
                    i += 1
                }
            } else if wav == nil {
                wav = args[i]
            }
            i += 1
        }
        let player = FeedbackPlayer(wavPath: wav, intensity: intensity)
        let lowerKind = kind.lowercased()
        if ["tick", "sound", "success", "success_sound", "error"].contains(lowerKind) {
            player.warmUp()
        }
        player.play(kind: kind, intensityOverride: intensity)
        let waitSeconds: TimeInterval
        switch lowerKind {
        case "success", "success_sound":
            waitSeconds = 0.65
        case "error":
            waitSeconds = 0.85
        case "tick", "sound":
            waitSeconds = 0.15
        case "alignment", "level", "generic", "success_haptic":
            waitSeconds = 0.12
        default:
            waitSeconds = wav != nil ? 0.15 : 0.0
        }
        if waitSeconds > 0 {
            RunLoop.current.run(until: Date().addingTimeInterval(waitSeconds))
        }
        writeJSON(["ok": true])

    case "notify":
        guard args.count >= 3 else { failExit("notify <title> <body> [reveal-path]") }
        let revealPath = args.count >= 4 ? args[3] : nil
        let res = FinderBridge.sendNotification(title: args[1], body: args[2], revealPath: revealPath)
        writeJSON(res)
        if (res["ok"] as? Bool) != true { exit(1) }

    default:
        printUsage()
        failExit("Unknown command: \(cmd)")
    }
}

runCLI()
