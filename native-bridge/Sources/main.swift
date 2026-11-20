import Foundation
import AppKit

func printUsage() {
    let usage = """
    flip-macos-bridge subcommands:
      monitor [--drag-key Down] [--idle-shortcut Option+Shift+Space] [--mouse-button -1] [--wav <path>]
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
      convert-av <input> <format> <output>
      create-iwork <pages|numbers|key> <output>
      feedback <tick|sound|alignment|level|generic|success|error> [wav-path]
      notify <title> <body> [reveal-path]
    """
    print(usage)
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
            default:
                break
            }
            i += 1
        }
        MonitorController.shared.run(dragKey: dragKey, idleShortcutSpec: idleShortcut, mouseButton: mouseButton, wavPath: wavPath)

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
        guard args.count >= 2 else { failExit("feedback <kind> [wav-path]") }
        let kind = args[1]
        let wav = args.count >= 3 ? args[2] : nil
        let player = FeedbackPlayer(wavPath: wav)
        player.play(kind: kind)
        let lowerKind = kind.lowercased()
        if wav != nil || lowerKind == "tick" || lowerKind == "sound" {
            RunLoop.current.run(until: Date().addingTimeInterval(0.15))
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
