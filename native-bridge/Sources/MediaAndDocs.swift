import Foundation
import AppKit
import PDFKit
import Vision
import CoreML
import AVFoundation
import QuickLookThumbnailing
import UniformTypeIdentifiers
import ImageIO

enum MediaAndDocs {
    // MARK: - Thumbnail via QuickLookThumbnailing
    static func generateThumbnail(path: String, size: CGFloat) -> [String: Any] {
        let url = URL(fileURLWithPath: path)
        guard FileManager.default.fileExists(atPath: path) else {
            return ["ok": false, "error": "File does not exist"]
        }

        let semaphore = DispatchSemaphore(value: 0)
        var resultImage: NSImage?

        let request = QLThumbnailGenerator.Request(
            fileAt: url,
            size: CGSize(width: size, height: size),
            scale: 2.0,
            representationTypes: .all
        )

        QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { rep, _ in
            if let rep = rep {
                resultImage = rep.nsImage
            }
            semaphore.signal()
        }

        _ = semaphore.wait(timeout: .now() + 1.5)
        let finalImage = resultImage ?? NSWorkspace.shared.icon(forFile: path)

        guard let tiff = finalImage.tiffRepresentation,
              let bitmap = NSBitmapImageRep(data: tiff),
              let pngData = bitmap.representation(using: .png, properties: [:]) else {
            return ["ok": false, "error": "Failed to encode thumbnail PNG"]
        }

        return [
            "ok": true,
            "pngBase64": pngData.base64EncodedString(),
            "width": Int(finalImage.size.width),
            "height": Int(finalImage.size.height)
        ]
    }

    // MARK: - Vision OCR
    static func recognizeText(cgImage: CGImage) throws -> String {
        var recognizedLines: [String] = []
        var visionError: Error?

        let request = VNRecognizeTextRequest { req, err in
            if let err = err {
                visionError = err
                return
            }
            guard let observations = req.results as? [VNRecognizedTextObservation] else { return }
            for obs in observations {
                if let top = obs.topCandidates(1).first {
                    recognizedLines.append(top.string)
                }
            }
        }
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        if #available(macOS 13.0, *) {
            request.automaticallyDetectsLanguage = true
        }
        // In CLI / non-windowed helper processes, ANE/GPU compute stages can stall.
        // Force the CPU compute device cleanly via macOS 14+ ComputeStage APIs.
        if #available(macOS 14.0, *) {
            if let stageDevices = try? request.supportedComputeStageDevices {
                for (stage, devices) in stageDevices {
                    if let cpuDevice = devices.first(where: {
                        if case .cpu = $0 { return true }
                        return false
                    }) {
                        request.setComputeDevice(cpuDevice, for: stage)
                    }
                }
            }
        } else {
            request.usesCPUOnly = true
        }

        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        try handler.perform([request])
        if let err = visionError {
            throw err
        }
        return recognizedLines.joined(separator: "\n")
    }

    static func ocrImageFile(path: String, outTxtPath: String?) throws -> [String: Any] {
        let url = URL(fileURLWithPath: path)
        guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
              let cgImg = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
            throw BridgeError.operationFailed("Could not decode image for OCR: \(path)")
        }
        let text = try recognizeText(cgImage: cgImg)
        if let outTxtPath = outTxtPath {
            try Data(text.utf8).write(to: URL(fileURLWithPath: outTxtPath), options: .atomic)
        }
        return ["ok": true, "text": text, "characters": text.count]
    }

    // MARK: - PDFKit + Vision OCR
    static func renderPDFPages(pdfPath: String, outDir: String, format: String, dpi: CGFloat = 144.0) throws -> [String: Any] {
        guard let doc = PDFDocument(url: URL(fileURLWithPath: pdfPath)) else {
            throw BridgeError.operationFailed("Failed to open PDF: \(pdfPath)")
        }
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw BridgeError.operationFailed("PDF has 0 pages")
        }
        if pageCount > 200 {
            throw BridgeError.operationFailed("PDF exceeds 200-page safety limit (\(pageCount) pages)")
        }

        let fmtUpper = format.uppercased()
        let ext: String
        let utType: UTType
        switch fmtUpper {
        case "PNG": ext = "png"; utType = .png
        case "JPG", "JPEG": ext = "jpg"; utType = .jpeg
        case "TIFF", "TIF": ext = "tiff"; utType = .tiff
        case "HEIC": ext = "heic"; utType = .heic
        case "BMP": ext = "bmp"; utType = .bmp
        case "GIF": ext = "gif"; utType = .gif
        default:
            throw BridgeError.invalidArguments("Unsupported PDF render format: \(format)")
        }

        try FileManager.default.createDirectory(atPath: outDir, withIntermediateDirectories: true)
        var outputs: [String] = []
        let scale = dpi / 72.0

        for idx in 0..<pageCount {
            guard let page = doc.page(at: idx) else { continue }
            let box = page.bounds(for: .mediaBox)
            let width = max(1, Int((box.width * scale).rounded()))
            let height = max(1, Int((box.height * scale).rounded()))

            guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
                  let ctx = CGContext(
                    data: nil,
                    width: width,
                    height: height,
                    bitsPerComponent: 8,
                    bytesPerRow: 0,
                    space: colorSpace,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                  ) else {
                throw BridgeError.operationFailed("Failed to allocate render context for page \(idx + 1)")
            }

            ctx.setFillColor(CGColor.white)
            ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
            ctx.scaleBy(x: scale, y: scale)
            page.draw(with: .mediaBox, to: ctx)

            guard let cgImage = ctx.makeImage() else {
                throw BridgeError.operationFailed("Failed to render page \(idx + 1)")
            }

            let outPath = (outDir as NSString).appendingPathComponent(String(format: "page-%03d.%@", idx + 1, ext))
            let outURL = URL(fileURLWithPath: outPath)
            guard let dest = CGImageDestinationCreateWithURL(outURL as CFURL, utType.identifier as CFString, 1, nil) else {
                throw BridgeError.operationFailed("Failed to create image destination for \(outPath)")
            }
            let props: CFDictionary = (fmtUpper == "JPG" || fmtUpper == "JPEG" || fmtUpper == "HEIC")
                ? ([kCGImageDestinationLossyCompressionQuality: 0.92] as CFDictionary)
                : ([:] as CFDictionary)
            CGImageDestinationAddImage(dest, cgImage, props)
            if !CGImageDestinationFinalize(dest) {
                throw BridgeError.operationFailed("Failed to finalize image file \(outPath)")
            }
            outputs.append(outPath)
        }

        return ["ok": true, "outputs": outputs, "pages": pageCount]
    }

    static func extractPDFText(pdfPath: String, outTxtPath: String) throws -> [String: Any] {
        guard let doc = PDFDocument(url: URL(fileURLWithPath: pdfPath)) else {
            throw BridgeError.operationFailed("Failed to open PDF: \(pdfPath)")
        }
        let pageCount = doc.pageCount
        if pageCount > 200 {
            throw BridgeError.operationFailed("PDF exceeds 200-page safety limit")
        }

        var parts: [String] = []
        var ocrPages = 0

        for idx in 0..<pageCount {
            guard let page = doc.page(at: idx) else { continue }
            let raw = (page.string ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if !raw.isEmpty {
                parts.append(raw)
            } else {
                // Scanned page -> Vision OCR fallback
                let box = page.bounds(for: .mediaBox)
                let scale: CGFloat = 200.0 / 72.0
                let w = max(1, Int((box.width * scale).rounded()))
                let h = max(1, Int((box.height * scale).rounded()))
                if let cs = CGColorSpace(name: CGColorSpace.sRGB),
                   let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
                    ctx.setFillColor(CGColor.white)
                    ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
                    ctx.scaleBy(x: scale, y: scale)
                    page.draw(with: .mediaBox, to: ctx)
                    if let cgImg = ctx.makeImage() {
                        let ocr = try recognizeText(cgImage: cgImg)
                        if !ocr.isEmpty {
                            parts.append(ocr)
                            ocrPages += 1
                        }
                    }
                }
            }
        }

        let combined = parts.joined(separator: "\n\n")
        try Data(combined.utf8).write(to: URL(fileURLWithPath: outTxtPath), options: .atomic)
        return ["ok": true, "pages": pageCount, "ocrPages": ocrPages, "characters": combined.count]
    }

    static func imagesToPDF(outPdfPath: String, imagePaths: [String]) throws -> [String: Any] {
        let pdf = PDFDocument()
        for (idx, path) in imagePaths.enumerated() {
            guard let img = NSImage(contentsOfFile: path), let page = PDFPage(image: img) else {
                throw BridgeError.operationFailed("Could not read image for PDF: \(path)")
            }
            pdf.insert(page, at: idx)
        }
        guard pdf.write(toFile: outPdfPath) else {
            throw BridgeError.operationFailed("Failed to write PDF: \(outPdfPath)")
        }
        return ["ok": true, "pages": pdf.pageCount, "output": outPdfPath]
    }

    static func mergePDFs(outPdfPath: String, pdfPaths: [String]) throws -> [String: Any] {
        let outDoc = PDFDocument()
        var cursor = 0
        for path in pdfPaths {
            guard let src = PDFDocument(url: URL(fileURLWithPath: path)) else {
                throw BridgeError.operationFailed("Failed to open PDF: \(path)")
            }
            for i in 0..<src.pageCount {
                if let page = src.page(at: i) {
                    outDoc.insert(page, at: cursor)
                    cursor += 1
                }
            }
        }
        guard outDoc.pageCount > 0, outDoc.write(toFile: outPdfPath) else {
            throw BridgeError.operationFailed("Failed to write merged PDF")
        }
        return ["ok": true, "pages": outDoc.pageCount, "output": outPdfPath]
    }

    // MARK: - Native Documents (NSAttributedString + A4 PDF + Apple iWork)
    static func convertDocument(inputPath: String, format: String, outputPath: String, pageSize: String = "a4") throws -> [String: Any] {
        let inURL = URL(fileURLWithPath: inputPath)
        let inExt = inURL.pathExtension.lowercased()
        let fmtUpper = format.uppercased()

        // Handle Apple iWork formats (.pages, .numbers, .key)
        if ["pages", "numbers", "key"].contains(inExt) {
            return try convertIWork(inputPath: inputPath, inExt: inExt, format: fmtUpper, outputPath: outputPath)
        }

        let attrString: NSAttributedString
        do {
            var docOpts: [NSAttributedString.DocumentReadingOptionKey: Any] = [:]
            if inExt == "txt" || inExt == "md" || inExt == "markdown" {
                docOpts[.documentType] = NSAttributedString.DocumentType.plain
                docOpts[.characterEncoding] = String.Encoding.utf8.rawValue
            }
            attrString = try NSAttributedString(url: inURL, options: docOpts, documentAttributes: nil)
        } catch {
            throw BridgeError.operationFailed("Failed to parse document \(inputPath): \(error.localizedDescription)")
        }

        if fmtUpper == "PDF" {
            return try renderAttributedStringToPDF(attrString, outputPath: outputPath, pageSize: pageSize)
        }

        let targetType: NSAttributedString.DocumentType
        switch fmtUpper {
        case "TXT": targetType = .plain
        case "RTF": targetType = .rtf
        case "HTML": targetType = .html
        case "DOC": targetType = .docFormat
        case "DOCX": targetType = .officeOpenXML
        case "ODT": targetType = .openDocument
        default:
            throw BridgeError.invalidArguments("Unsupported native document target format: \(format)")
        }

        let range = NSRange(location: 0, length: attrString.length)
        let data = try attrString.data(
            from: range,
            documentAttributes: [
                .documentType: targetType,
                .characterEncoding: String.Encoding.utf8.rawValue
            ]
        )
        try data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        return ["ok": true, "output": outputPath]
    }

    private static func renderAttributedStringToPDF(_ attr: NSAttributedString, outputPath: String, pageSize: String) throws -> [String: Any] {
        // A4 = 595.28 x 841.89 pt, Letter = 612 x 792 pt
        let size = pageSize.lowercased() == "letter"
            ? CGSize(width: 612, height: 792)
            : CGSize(width: 595.28, height: 841.89)
        let margin: CGFloat = 48

        let mutable = NSMutableAttributedString(attributedString: attr)
        let fullRange = NSRange(location: 0, length: mutable.length)
        mutable.enumerateAttribute(.font, in: fullRange) { val, range, _ in
            if val == nil {
                mutable.addAttribute(.font, value: NSFont.systemFont(ofSize: 11.5), range: range)
            }
        }
        mutable.enumerateAttribute(.foregroundColor, in: fullRange) { val, range, _ in
            if val == nil {
                mutable.addAttribute(.foregroundColor, value: NSColor.black, range: range)
            }
        }

        let pdfData = NSMutableData()
        guard let consumer = CGDataConsumer(data: pdfData as CFMutableData) else {
            throw BridgeError.operationFailed("Failed to allocate PDF data consumer")
        }
        var mediaBox = CGRect(origin: .zero, size: size)
        guard let pdfCtx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
            throw BridgeError.operationFailed("Failed to create PDF CGContext")
        }

        let framesetter = CTFramesetterCreateWithAttributedString(mutable as CFAttributedString)
        let textRect = CGRect(
            x: margin,
            y: margin,
            width: size.width - margin * 2,
            height: size.height - margin * 2
        )

        var currentIndex: CFIndex = 0
        let totalLength = mutable.length
        var pageCount = 0

        repeat {
            pageCount += 1
            if pageCount > 200 {
                throw BridgeError.operationFailed("PDF output exceeds 200-page safety limit")
            }
            pdfCtx.beginPDFPage(nil)
            pdfCtx.setFillColor(CGColor.white)
            pdfCtx.fill(mediaBox)

            let path = CGPath(rect: textRect, transform: nil)
            let frame = CTFramesetterCreateFrame(framesetter, CFRangeMake(currentIndex, 0), path, nil)
            CTFrameDraw(frame, pdfCtx)
            pdfCtx.endPDFPage()

            let visibleRange = CTFrameGetVisibleStringRange(frame)
            if visibleRange.length <= 0 {
                break
            }
            currentIndex += visibleRange.length
        } while currentIndex < totalLength

        pdfCtx.closePDF()

        let bytes = pdfData as Data
        guard !bytes.isEmpty else {
            throw BridgeError.operationFailed("PDF rendering produced empty output")
        }
        try bytes.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        return ["ok": true, "output": outputPath, "pages": pageCount]
    }

    private static func convertIWork(inputPath: String, inExt: String, format: String, outputPath: String) throws -> [String: Any] {
        let appName: String
        let appleFormat: String
        switch inExt {
        case "pages":
            appName = "Pages"
            switch format {
            case "PDF": appleFormat = "PDF"
            case "DOCX": appleFormat = "Microsoft Word"
            case "EPUB": appleFormat = "EPUB"
            case "RTF": appleFormat = "RTF"
            case "TXT": appleFormat = "unformatted text"
            default: throw BridgeError.invalidArguments("Pages cannot export to \(format)")
            }
        case "numbers":
            appName = "Numbers"
            switch format {
            case "PDF": appleFormat = "PDF"
            case "XLSX": appleFormat = "Microsoft Excel"
            case "CSV": appleFormat = "CSV"
            default: throw BridgeError.invalidArguments("Numbers cannot export to \(format)")
            }
        case "key":
            appName = "Keynote"
            switch format {
            case "PDF": appleFormat = "PDF"
            case "PPTX": appleFormat = "Microsoft PowerPoint"
            default: throw BridgeError.invalidArguments("Keynote cannot export to \(format)")
            }
        default:
            throw BridgeError.invalidArguments("Unknown iWork extension: \(inExt)")
        }

        let bundleId = "com.apple.iWork.\(appName)"
        if NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) != nil {
            let escapedIn = inputPath.replacingOccurrences(of: "\"", with: "\\\"")
            let escapedOut = outputPath.replacingOccurrences(of: "\"", with: "\\\"")
            let script = """
            tell application "\(appName)"
                set doc to open POSIX file "\(escapedIn)"
                export doc to POSIX file "\(escapedOut)" as \(appleFormat)
                close doc saving no
            end tell
            """
            let (_, err) = FinderBridge.runAppleScript(script)
            if err == nil && FileManager.default.fileExists(atPath: outputPath) {
                return ["ok": true, "engine": appName, "output": outputPath]
            }
        }

        // Fallback: if format is PDF and QuickLook/Preview.pdf exists inside iWork package/zip
        if format == "PDF" {
            let previewInsidePkg = (inputPath as NSString).appendingPathComponent("QuickLook/Preview.pdf")
            if FileManager.default.fileExists(atPath: previewInsidePkg) {
                try FileManager.default.copyItem(atPath: previewInsidePkg, toPath: outputPath)
                return ["ok": true, "engine": "iwork-preview", "output": outputPath]
            }
            let unzip = Process()
            unzip.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
            unzip.arguments = ["-p", inputPath, "QuickLook/Preview.pdf"]
            let pipe = Pipe()
            unzip.standardOutput = pipe
            unzip.standardError = FileHandle.nullDevice
            try? unzip.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            unzip.waitUntilExit()
            if unzip.terminationStatus == 0 && !data.isEmpty {
                try data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
                return ["ok": true, "engine": "iwork-preview", "output": outputPath]
            }
        }

        throw BridgeError.operationFailed("\(appName) is not installed or failed to export \(inExt) to \(format)")
    }

    // MARK: - Image conversion via ImageIO (HEIC, AVIF, ICNS, PNG, JPG, TIFF, BMP, GIF)
    private static func normalizeOrientedImage(_ cgImage: CGImage, orientation: UInt32, flattenAlpha: Bool) -> CGImage? {
        let w = cgImage.width
        let h = cgImage.height
        let swapAxes = orientation >= 5 && orientation <= 8
        let outW = swapAxes ? h : w
        let outH = swapAxes ? w : h

        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        let bitmapInfo: UInt32 = flattenAlpha
            ? CGImageAlphaInfo.noneSkipLast.rawValue
            : CGImageAlphaInfo.premultipliedLast.rawValue

        guard let ctx = CGContext(
            data: nil,
            width: outW,
            height: outH,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: colorSpace,
            bitmapInfo: bitmapInfo
        ) else {
            return nil
        }

        if flattenAlpha {
            ctx.setFillColor(CGColor.white)
            ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
        }

        let transform: CGAffineTransform
        switch orientation {
        case 2:
            transform = CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: CGFloat(w), ty: 0)
        case 3:
            transform = CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: CGFloat(w), ty: CGFloat(h))
        case 4:
            transform = CGAffineTransform(a: 1, b: 0, c: 0, d: -1, tx: 0, ty: CGFloat(h))
        case 5:
            transform = CGAffineTransform(a: 0, b: -1, c: -1, d: 0, tx: CGFloat(h), ty: CGFloat(w))
        case 6:
            transform = CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: CGFloat(w))
        case 7:
            transform = CGAffineTransform(a: 0, b: 1, c: 1, d: 0, tx: 0, ty: 0)
        case 8:
            transform = CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: CGFloat(h), ty: 0)
        default:
            transform = .identity
        }

        ctx.concatenate(transform)
        ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: w, height: h))
        return ctx.makeImage()
    }

    static func convertImage(inputPath: String, format: String, outputPath: String, quality: Double = 0.92) throws -> [String: Any] {
        let inURL = URL(fileURLWithPath: inputPath)
        guard let src = CGImageSourceCreateWithURL(inURL as CFURL, nil),
              let rawImage = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
            throw BridgeError.operationFailed("Could not decode image: \(inputPath)")
        }
        if Int64(rawImage.width) * Int64(rawImage.height) > 40_000_000 {
            throw BridgeError.operationFailed("Image exceeds 40 megapixel safety limit")
        }

        let srcProps = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any]
        let orientation = (srcProps?[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value ?? 1

        let fmtUpper = format.uppercased()
        let utType: UTType
        switch fmtUpper {
        case "PNG": utType = .png
        case "JPG", "JPEG": utType = .jpeg
        case "HEIC": utType = .heic
        case "TIFF", "TIF": utType = .tiff
        case "BMP": utType = .bmp
        case "GIF": utType = .gif
        case "ICNS": utType = .icns
        default:
            throw BridgeError.invalidArguments("Unsupported ImageIO output format: \(format)")
        }

        let flattenAlpha = (fmtUpper == "JPG" || fmtUpper == "JPEG" || fmtUpper == "BMP")
        var finalImage: CGImage
        if orientation != 1 || flattenAlpha {
            finalImage = normalizeOrientedImage(rawImage, orientation: orientation, flattenAlpha: flattenAlpha) ?? rawImage
        } else {
            finalImage = rawImage
        }

        if fmtUpper == "ICNS" {
            let maxDim = max(finalImage.width, finalImage.height)
            let validSizes = [16, 32, 64, 128, 256, 512, 1024]
            let targetSide = validSizes.first(where: { $0 >= maxDim }) ?? 512
            if finalImage.width != targetSide || finalImage.height != targetSide {
                if let cs = CGColorSpace(name: CGColorSpace.sRGB),
                   let ctx = CGContext(
                    data: nil,
                    width: targetSide,
                    height: targetSide,
                    bitsPerComponent: 8,
                    bytesPerRow: 0,
                    space: cs,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                   ) {
                    let scale = min(CGFloat(targetSide) / CGFloat(finalImage.width), CGFloat(targetSide) / CGFloat(finalImage.height))
                    let drawW = CGFloat(finalImage.width) * scale
                    let drawH = CGFloat(finalImage.height) * scale
                    let drawRect = CGRect(
                        x: (CGFloat(targetSide) - drawW) / 2.0,
                        y: (CGFloat(targetSide) - drawH) / 2.0,
                        width: drawW,
                        height: drawH
                    )
                    ctx.interpolationQuality = .high
                    ctx.draw(finalImage, in: drawRect)
                    if let scaled = ctx.makeImage() {
                        finalImage = scaled
                    }
                }
            }
        }

        let outURL = URL(fileURLWithPath: outputPath)
        guard let dest = CGImageDestinationCreateWithURL(outURL as CFURL, utType.identifier as CFString, 1, nil) else {
            throw BridgeError.operationFailed("Failed to create ImageIO destination for \(fmtUpper)")
        }
        var props: [CFString: Any] = [
            kCGImagePropertyOrientation: 1
        ]
        if fmtUpper == "JPG" || fmtUpper == "JPEG" || fmtUpper == "HEIC" {
            props[kCGImageDestinationLossyCompressionQuality] = quality
        }
        CGImageDestinationAddImage(dest, finalImage, props as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw BridgeError.operationFailed("ImageIO failed to write \(outputPath)")
        }
        return ["ok": true, "output": outputPath, "width": finalImage.width, "height": finalImage.height]
    }

    // MARK: - AVFoundation Native Media Export
    static func convertAV(inputPath: String, format: String, outputPath: String) throws -> [String: Any] {
        let inURL = URL(fileURLWithPath: inputPath)
        let outURL = URL(fileURLWithPath: outputPath)
        let fmtUpper = format.uppercased()

        let preset: String
        let fileType: AVFileType
        switch fmtUpper {
        case "M4A":
            preset = AVAssetExportPresetAppleM4A
            fileType = .m4a
        case "MP4":
            preset = AVAssetExportPresetHighestQuality
            fileType = .mp4
        case "MOV":
            preset = AVAssetExportPresetHighestQuality
            fileType = .mov
        case "M4V":
            preset = AVAssetExportPresetHighestQuality
            fileType = .m4v
        default:
            throw BridgeError.invalidArguments("AVFoundation native export does not target \(format)")
        }

        let asset = AVURLAsset(url: inURL)
        guard let exporter = AVAssetExportSession(asset: asset, presetName: preset) else {
            throw BridgeError.operationFailed("Could not initialize AVAssetExportSession for \(inputPath)")
        }
        exporter.shouldOptimizeForNetworkUse = true
        let sem = DispatchSemaphore(value: 0)
        var exportError: Error?
        Task {
            do {
                try await exporter.export(to: outURL, as: fileType)
            } catch {
                exportError = error
            }
            sem.signal()
        }
        sem.wait()

        if let err = exportError {
            throw BridgeError.operationFailed(err.localizedDescription)
        }
        return ["ok": true, "output": outputPath]
    }
}
