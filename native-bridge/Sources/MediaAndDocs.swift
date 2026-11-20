import Foundation
import AppKit
import PDFKit
import Vision
import CoreML
import CoreImage
import Quartz
import AVFoundation
import AudioToolbox
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

    private static let maxImagePixels: Int64 = 40_000_000
    private static let maxContainerBytes: Int = 256 * 1024 * 1024
    private static let maxHtmlInputBytes: Int = 20 * 1024 * 1024
    private static let maxSvgInputBytes: Int = 20 * 1024 * 1024

    private static func inspectImagePropertiesPreDecode(
        _ src: CGImageSource,
        path: String
    ) throws -> (orientation: UInt32, props: [CFString: Any]?) {
        let srcProps = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any]
        if let pw = (srcProps?[kCGImagePropertyPixelWidth] as? NSNumber)?.int64Value,
           let ph = (srcProps?[kCGImagePropertyPixelHeight] as? NSNumber)?.int64Value {
            if pw <= 0 || ph <= 0 {
                throw BridgeError.operationFailed("Image has invalid zero dimensions: \(path)")
            }
            let (pixels, overflow) = pw.multipliedReportingOverflow(by: ph)
            if overflow || pixels > maxImagePixels {
                throw BridgeError.operationFailed("Image exceeds 40 megapixel safety limit (\(pw)x\(ph))")
            }
        }
        let orientation = (srcProps?[kCGImagePropertyOrientation] as? NSNumber)?.uint32Value ?? 1
        return (orientation, srcProps)
    }

    // MARK: - Vision OCR
    private static func performRecognizeText(
        cgImage: CGImage,
        orientation: CGImagePropertyOrientation,
        level: VNRequestTextRecognitionLevel
    ) throws -> String {
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
        request.recognitionLevel = level
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

        let handler = VNImageRequestHandler(cgImage: cgImage, orientation: orientation, options: [:])
        try handler.perform([request])
        if let err = visionError {
            throw err
        }
        return recognizedLines.joined(separator: "\n")
    }

    private static func upscale2xForOCR(_ cgImage: CGImage) -> CGImage? {
        let w = cgImage.width
        let h = cgImage.height
        guard w > 0 && h > 0 && max(w, h) <= 3200 else { return nil }
        let outW = w * 2
        let outH = h * 2
        let (pixels, overflow) = Int64(outW).multipliedReportingOverflow(by: Int64(outH))
        guard !overflow && pixels <= maxImagePixels else { return nil }
        guard let cs = CGColorSpace(name: CGColorSpace.sRGB),
              let ctx = CGContext(
                data: nil,
                width: outW,
                height: outH,
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: cs,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
              ) else {
            return nil
        }
        ctx.setFillColor(CGColor.white)
        ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
        ctx.interpolationQuality = .high
        ctx.draw(cgImage, in: CGRect(x: 0, y: 0, width: outW, height: outH))
        return ctx.makeImage()
    }

    static func recognizeText(
        cgImage: CGImage,
        orientation: CGImagePropertyOrientation = .up
    ) throws -> String {
        let accurateText = try performRecognizeText(cgImage: cgImage, orientation: orientation, level: .accurate)
        if !accurateText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return accurateText
        }
        // Vision .accurate can return 0 observations on smaller body text (e.g. 144 DPI full-page docs);
        // retry .accurate on a 2x white-backed upscale (no .fast hallucinations on geometric shapes).
        if let upscaled = upscale2xForOCR(cgImage) {
            return try performRecognizeText(cgImage: upscaled, orientation: orientation, level: .accurate)
        }
        return accurateText
    }

    /// One entry per whitespace-separated word, in Vision reading order, with the box converted to
    /// normalised top-left origin coordinates in the display (EXIF-oriented) image space.
    private static func wordBoxes(for candidate: VNRecognizedText) -> [[String: Any]] {
        let text = candidate.string
        var words: [[String: Any]] = []
        var index = text.startIndex
        while index < text.endIndex {
            while index < text.endIndex && text[index].isWhitespace {
                index = text.index(after: index)
            }
            guard index < text.endIndex else { break }
            let start = index
            while index < text.endIndex && !text[index].isWhitespace {
                index = text.index(after: index)
            }
            let wordRange = start..<index
            guard let observation = try? candidate.boundingBox(for: wordRange) else { continue }
            let box = observation.boundingBox
            // Vision's origin is bottom-left; the contract wants top-left.
            let y = 1.0 - (box.origin.y + box.size.height)
            words.append([
                "text": String(text[start..<index]),
                "box": [box.origin.x, y, box.size.width, box.size.height]
            ])
        }
        return words
    }

    private static func performRecognizeWordBoxes(
        cgImage: CGImage,
        orientation: CGImagePropertyOrientation,
        level: VNRequestTextRecognitionLevel,
        usesLanguageCorrection: Bool
    ) throws -> [[[String: Any]]] {
        var recognizedLines: [[[String: Any]]] = []
        var visionError: Error?

        let request = VNRecognizeTextRequest { req, err in
            if let err = err {
                visionError = err
                return
            }
            guard let observations = req.results as? [VNRecognizedTextObservation] else { return }
            for obs in observations {
                guard let top = obs.topCandidates(1).first else { continue }
                let words = wordBoxes(for: top)
                if !words.isEmpty {
                    recognizedLines.append(words)
                }
            }
        }
        request.recognitionLevel = level
        request.usesLanguageCorrection = usesLanguageCorrection
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

        let handler = VNImageRequestHandler(cgImage: cgImage, orientation: orientation, options: [:])
        try handler.perform([request])
        if let err = visionError {
            throw err
        }
        return recognizedLines
    }

    private static func recognizeWordBoxes(
        cgImage: CGImage,
        orientation: CGImagePropertyOrientation
    ) throws -> [[[String: Any]]] {
        // Language correction rewrites codes ("0" -> "O"), which would make a redaction tool miss
        // an IBAN or a tax code: keep it off and take the recogniser's raw words.
        let accurateLines = try performRecognizeWordBoxes(
            cgImage: cgImage,
            orientation: orientation,
            level: .accurate,
            usesLanguageCorrection: false
        )
        let hasText = accurateLines.contains { line in
            line.contains { word in
                let text = word["text"] as? String ?? ""
                return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
        }
        if hasText {
            return accurateLines
        }
        // Same retry as recognizeText: .accurate can return nothing on smaller body text.
        if let upscaled = upscale2xForOCR(cgImage) {
            return try performRecognizeWordBoxes(
                cgImage: upscaled,
                orientation: orientation,
                level: .accurate,
                usesLanguageCorrection: false
            )
        }
        return accurateLines
    }

    private static func sanitizeSVGForOfflineRasterization(_ svg: String) -> String {
        var out = svg
        let stripPatterns = [
            "<!DOCTYPE\\b[^>]*\\[[\\s\\S]*?\\]\\s*>",
            "<!DOCTYPE\\b[^>]*>",
            "<!ENTITY\\b[^>]*>",
            "<\\?xml-stylesheet\\b[\\s\\S]*?\\?>",
            "<script\\b[^>]*>[\\s\\S]*?</script>",
            "<script\\b[^>]*>",
            "<foreignObject\\b[^>]*>[\\s\\S]*?</foreignObject>",
            "<foreignObject\\b[^>]*>",
            "<iframe\\b[^>]*>[\\s\\S]*?</iframe>",
            "<iframe\\b[^>]*>",
            "<object\\b[^>]*>[\\s\\S]*?</object>",
            "<embed\\b[^>]*>"
        ]
        for pattern in stripPatterns {
            if let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
                out = regex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "")
            }
        }

        // Preserve local SVG fragment refs (#id) and safe inline raster data:image/ URIs; block file://, http(s)://, and relative paths.
        let transparent1x1 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="
        let safePrefix = "(?!\\s*(?:#|data:image/(?:png|jpe?g|gif|webp|bmp|tiff)\\b))"
        let refDoubleQuoted = "((?:\\bxlink:)?\\b(?:href|src|srcset|poster|data)\\s*=\\s*\")\(safePrefix)([^\"]*)(\")"
        let refSingleQuoted = "((?:\\bxlink:)?\\b(?:href|src|srcset|poster|data)\\s*=\\s*')\(safePrefix)([^']*)(')"
        let refUnquoted = "((?:\\bxlink:)?\\b(?:href|src|srcset|poster|data)\\s*=\\s*)(?!\"|'|\(safePrefix.dropFirst(3).dropLast(1)))([^\\s>]+)"
        for (idx, pattern) in [refDoubleQuoted, refSingleQuoted, refUnquoted].enumerated() {
            if let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
                let template = idx == 2 ? "$1\"\(transparent1x1)\"" : "$1\(transparent1x1)$3"
                out = regex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: template)
            }
        }

        if let cssImportRegex = try? NSRegularExpression(pattern: "@import\\b[^;<\\n]*[;\\n]?", options: [.caseInsensitive]) {
            out = cssImportRegex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "")
        }
        if let cssUrlRegex = try? NSRegularExpression(pattern: "url\\(\\s*(?![\"']?(?:#|data:image/(?:png|jpe?g|gif|webp|bmp|tiff)\\b))[^)]*\\)", options: [.caseInsensitive]) {
            out = cssUrlRegex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "url()")
        }
        return out
    }

    private static func loadSanitizedSVGData(path: String) throws -> Data {
        let url = URL(fileURLWithPath: path)
        if let attrs = try? FileManager.default.attributesOfItem(atPath: path) {
            if let ftype = attrs[.type] as? FileAttributeType, ftype == .typeSymbolicLink {
                throw BridgeError.operationFailed("Refusing symlinked SVG image: \(path)")
            }
            let size = (attrs[.size] as? NSNumber)?.intValue ?? 0
            if size <= 0 {
                throw BridgeError.operationFailed("SVG image is empty: \(path)")
            }
            if size > maxSvgInputBytes {
                throw BridgeError.operationFailed("SVG image exceeds 20 MiB safety limit")
            }
        }
        let rawData = try Data(contentsOf: url)
        if rawData.count > maxSvgInputBytes {
            throw BridgeError.operationFailed("SVG image exceeds 20 MiB safety limit")
        }
        let rawStr: String
        if let utf8 = String(data: rawData, encoding: .utf8) {
            rawStr = utf8
        } else if let latin1 = String(data: rawData, encoding: .isoLatin1) {
            rawStr = latin1
        } else {
            throw BridgeError.operationFailed("Could not decode SVG text encoding: \(path)")
        }
        let sanitized = sanitizeSVGForOfflineRasterization(rawStr)
        return Data(sanitized.utf8)
    }

    private static func rasterizeSVGToCGImage(path: String) throws -> CGImage {
        let sanitizedData = try loadSanitizedSVGData(path: path)
        if let nsImg = NSImage(data: sanitizedData), !nsImg.representations.isEmpty {
            let srcSize = nsImg.size
            let rawW = srcSize.width > 0 ? srcSize.width : 1024.0
            let rawH = srcSize.height > 0 ? srcSize.height : 1024.0
            let maxSide = max(rawW, rawH)
            let scale: CGFloat = maxSide < 1024.0 ? (1024.0 / maxSide) : 1.0
            var outW = max(1, Int((rawW * scale).rounded()))
            var outH = max(1, Int((rawH * scale).rounded()))
            let (pixels, overflow) = Int64(outW).multipliedReportingOverflow(by: Int64(outH))
            if overflow || pixels > maxImagePixels {
                let downScale = sqrt(Double(maxImagePixels) / (Double(outW) * Double(outH))) * 0.99
                outW = max(1, Int(floor(Double(outW) * downScale)))
                outH = max(1, Int(floor(Double(outH) * downScale)))
            }
            if let cs = CGColorSpace(name: CGColorSpace.sRGB),
               let ctx = CGContext(
                data: nil,
                width: outW,
                height: outH,
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: cs,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
               ) {
                let nsCtx = NSGraphicsContext(cgContext: ctx, flipped: false)
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = nsCtx
                nsImg.draw(
                    in: NSRect(x: 0, y: 0, width: outW, height: outH),
                    from: .zero,
                    operation: .copy,
                    fraction: 1.0
                )
                NSGraphicsContext.restoreGraphicsState()
                if let cgImg = ctx.makeImage() {
                    return cgImg
                }
            }
        }

        let tmpURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("flip-svg-\(UUID().uuidString).svg")
        try sanitizedData.write(to: tmpURL, options: .atomic)
        defer { try? FileManager.default.removeItem(at: tmpURL) }

        let semaphore = DispatchSemaphore(value: 0)
        var qlImage: CGImage?
        let request = QLThumbnailGenerator.Request(
            fileAt: tmpURL,
            size: CGSize(width: 1024, height: 1024),
            scale: 1.0,
            representationTypes: .thumbnail
        )
        QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { rep, _ in
            qlImage = rep?.cgImage
            semaphore.signal()
        }
        _ = semaphore.wait(timeout: .now() + 3.0)
        if let cgImg = qlImage {
            let (pixels, overflow) = Int64(cgImg.width).multipliedReportingOverflow(by: Int64(cgImg.height))
            if cgImg.width > 0 && cgImg.height > 0 && !overflow && pixels <= maxImagePixels {
                return cgImg
            }
        }

        throw BridgeError.operationFailed("Could not decode image: \(path)")
    }

    private static func decodeCGImageAndProps(path: String) throws -> (image: CGImage, orientation: UInt32, props: [CFString: Any]?) {
        let url = URL(fileURLWithPath: path)
        if let src = CGImageSourceCreateWithURL(url as CFURL, nil) {
            let (orientation, props) = try inspectImagePropertiesPreDecode(src, path: path)
            if let cgImg = CGImageSourceCreateImageAtIndex(src, 0, nil) {
                let (pixels, overflow) = Int64(cgImg.width).multipliedReportingOverflow(by: Int64(cgImg.height))
                if cgImg.width <= 0 || cgImg.height <= 0 || overflow || pixels > maxImagePixels {
                    throw BridgeError.operationFailed("Image exceeds 40 megapixel safety limit or has zero size")
                }
                return (cgImg, orientation, props)
            }
        }
        if url.pathExtension.lowercased() == "svg" {
            let cgImg = try rasterizeSVGToCGImage(path: path)
            return (cgImg, 1, nil)
        }
        throw BridgeError.operationFailed("Could not open or decode image: \(path)")
    }

    static func ocrImageFile(path: String, outTxtPath: String?) throws -> [String: Any] {
        let (decodedImg, rawOrientation, _) = try decodeCGImageAndProps(path: path)
        let isSvg = URL(fileURLWithPath: path).pathExtension.lowercased() == "svg"
        let cgImg = isSvg
            ? (normalizeOrientedImage(decodedImg, orientation: 1, flattenAlpha: true) ?? decodedImg)
            : decodedImg

        let cgOrientation = CGImagePropertyOrientation(rawValue: rawOrientation) ?? .up
        let text = try recognizeText(cgImage: cgImg, orientation: cgOrientation)
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw BridgeError.operationFailed("Vision OCR found no readable text in image")
        }
        if let outTxtPath = outTxtPath {
            try Data(text.utf8).write(to: URL(fileURLWithPath: outTxtPath), options: .atomic)
        }
        return ["ok": true, "text": text, "characters": text.count]
    }

    static func ocrBoxes(path: String) throws -> [String: Any] {
        let (decodedImg, rawOrientation, _) = try decodeCGImageAndProps(path: path)
        let isSvg = URL(fileURLWithPath: path).pathExtension.lowercased() == "svg"
        let cgImg = isSvg
            ? (normalizeOrientedImage(decodedImg, orientation: 1, flattenAlpha: true) ?? decodedImg)
            : decodedImg

        let cgOrientation = CGImagePropertyOrientation(rawValue: rawOrientation) ?? .up
        let lines = try recognizeWordBoxes(cgImage: cgImg, orientation: cgOrientation)

        // The pipeline works on the stored pixels; the reported size is the upright one.
        let swapAxes = rawOrientation >= 5 && rawOrientation <= 8
        let width = swapAxes ? cgImg.height : cgImg.width
        let height = swapAxes ? cgImg.width : cgImg.height

        return [
            "ok": true,
            "width": width,
            "height": height,
            "lines": lines.map { ["words": $0] }
        ]
    }

    // MARK: - PDFKit + Vision OCR
    private static func renderPageToCGImage(_ page: PDFPage, dpi: CGFloat, box: PDFDisplayBox = .mediaBox) throws -> CGImage {
        let scale = dpi / 72.0
        let mediaBox = page.bounds(for: box)
        let transform = page.transform(for: box)
        let displayBox = mediaBox.applying(transform)

        let width = max(1, Int((displayBox.width * scale).rounded()))
        let height = max(1, Int((displayBox.height * scale).rounded()))

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
            throw BridgeError.operationFailed("Failed to allocate PDF page render context")
        }

        ctx.setFillColor(CGColor.white)
        ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
        ctx.scaleBy(x: scale, y: scale)
        ctx.translateBy(x: -displayBox.origin.x, y: -displayBox.origin.y)
        page.draw(with: box, to: ctx)

        guard let cgImage = ctx.makeImage() else {
            throw BridgeError.operationFailed("Failed to render PDF page to image")
        }
        return cgImage
    }

    static func renderPDFPages(pdfPath: String, outDir: String, format: String, dpi: CGFloat = 144.0) throws -> [String: Any] {
        guard let doc = PDFDocument(url: URL(fileURLWithPath: pdfPath)) else {
            throw BridgeError.operationFailed("Failed to open PDF: \(pdfPath)")
        }
        if doc.isLocked {
            throw BridgeError.operationFailed("PDF is password-protected or encrypted")
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

        for idx in 0..<pageCount {
            guard let page = doc.page(at: idx) else { continue }
            let cgImage = try renderPageToCGImage(page, dpi: dpi)

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
        if doc.isLocked {
            throw BridgeError.operationFailed("PDF is password-protected or encrypted")
        }
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw BridgeError.operationFailed("PDF has 0 pages")
        }
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
                // Scanned page -> Vision OCR fallback (with full rotation & MediaBox origin handling)
                if let cgImg = try? renderPageToCGImage(page, dpi: 200.0) {
                    let ocr = try recognizeText(cgImage: cgImg)
                    if !ocr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        parts.append(ocr)
                        ocrPages += 1
                    }
                }
            }
        }

        let combined = parts.joined(separator: "\n\n")
        guard !combined.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw BridgeError.operationFailed("PDF contains no extractable text; Vision OCR found no readable text")
        }
        try Data(combined.utf8).write(to: URL(fileURLWithPath: outTxtPath), options: .atomic)
        return ["ok": true, "pages": pageCount, "ocrPages": ocrPages, "characters": combined.count]
    }

    static func imagesToPDF(outPdfPath: String, imagePaths: [String]) throws -> [String: Any] {
        guard !imagePaths.isEmpty && imagePaths.count <= 128 else {
            throw BridgeError.invalidArguments("Select between 1 and 128 images for PDF")
        }
        guard !FileManager.default.fileExists(atPath: outPdfPath),
              (try? FileManager.default.attributesOfItem(atPath: outPdfPath)) == nil else {
            throw BridgeError.operationFailed("Refusing to overwrite existing output PDF: \(outPdfPath)")
        }

        // Pass 1: Pre-decode dimension & format validation across all inputs before decoding pixels
        enum PreparedPDFImage {
            case cgSource(src: CGImageSource, path: String, orientation: UInt32)
            case svgPath(path: String)
        }
        var preparedSources: [PreparedPDFImage] = []
        preparedSources.reserveCapacity(imagePaths.count)
        for path in imagePaths {
            let url = URL(fileURLWithPath: path)
            if url.pathExtension.lowercased() == "svg" {
                let sanitizedData = try loadSanitizedSVGData(path: path)
                guard let nsImg = NSImage(data: sanitizedData), !nsImg.representations.isEmpty else {
                    throw BridgeError.operationFailed("Could not open SVG image for PDF: \(path)")
                }
                preparedSources.append(.svgPath(path: path))
            } else {
                guard let src = CGImageSourceCreateWithURL(url as CFURL, nil) else {
                    throw BridgeError.operationFailed("Could not open image for PDF: \(path)")
                }
                let (orientation, _) = try inspectImagePropertiesPreDecode(src, path: path)
                preparedSources.append(.cgSource(src: src, path: path, orientation: orientation))
            }
        }

        // A4 595 x 842 pt with 30 pt margins aspect-fit (matching FLIP-windows ImagePdf)
        let pdfData = NSMutableData()
        guard let consumer = CGDataConsumer(data: pdfData as CFMutableData) else {
            throw BridgeError.operationFailed("Failed to allocate PDF data consumer")
        }
        var mediaBox = CGRect(x: 0, y: 0, width: 595, height: 842)
        guard let pdfCtx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
            throw BridgeError.operationFailed("Failed to create PDF CGContext")
        }

        let margin: CGFloat = 30
        let availW = mediaBox.width - 2 * margin
        let availH = mediaBox.height - 2 * margin

        // Pass 2: Decode, orient, draw, and release one image at a time inside autoreleasepool
        for item in preparedSources {
            try autoreleasepool {
                let cgImg: CGImage
                switch item {
                case .cgSource(let src, let path, let orientation):
                    guard let rawImg = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
                        throw BridgeError.operationFailed("Could not decode image for PDF: \(path)")
                    }
                    let (pixels, overflow) = Int64(rawImg.width).multipliedReportingOverflow(by: Int64(rawImg.height))
                    if rawImg.width <= 0 || rawImg.height <= 0 || overflow || pixels > maxImagePixels {
                        throw BridgeError.operationFailed("Image exceeds 40 megapixel safety limit or has zero size: \(path)")
                    }
                    if orientation != 1 {
                        cgImg = normalizeOrientedImage(rawImg, orientation: orientation, flattenAlpha: false) ?? rawImg
                    } else {
                        cgImg = rawImg
                    }
                case .svgPath(let path):
                    cgImg = try rasterizeSVGToCGImage(path: path)
                }

                pdfCtx.beginPDFPage(nil)
                pdfCtx.setFillColor(CGColor.white)
                pdfCtx.fill(mediaBox)

                let imgW = CGFloat(cgImg.width)
                let imgH = CGFloat(cgImg.height)
                let scale = min(availW / imgW, availH / imgH)
                let drawW = imgW * scale
                let drawH = imgH * scale
                let drawRect = CGRect(
                    x: (mediaBox.width - drawW) / 2.0,
                    y: (mediaBox.height - drawH) / 2.0,
                    width: drawW,
                    height: drawH
                )
                pdfCtx.interpolationQuality = .high
                pdfCtx.draw(cgImg, in: drawRect)
                pdfCtx.endPDFPage()
            }
        }
        pdfCtx.closePDF()

        let bytes = pdfData as Data
        guard !bytes.isEmpty else {
            throw BridgeError.operationFailed("Failed to generate PDF from images")
        }
        try bytes.write(to: URL(fileURLWithPath: outPdfPath), options: .withoutOverwriting)
        return ["ok": true, "pages": preparedSources.count, "output": outPdfPath]
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

    private static func resolveReduceFileSizeFilter() -> QuartzFilter? {
        let systemFilterURL = URL(fileURLWithPath: "/System/Library/Filters/Reduce File Size.qfilter")
        if FileManager.default.fileExists(atPath: systemFilterURL.path),
           let filter = QuartzFilter(url: systemFilterURL) {
            return filter
        }
        let available = (QuartzFilterManager.filters(inDomains: nil) as? [QuartzFilter]) ?? []
        if let byFilename = available.first(where: { $0.url()?.lastPathComponent == "Reduce File Size.qfilter" }) {
            return byFilename
        }
        if let byName = available.first(where: { ($0.localizedName() ?? "").lowercased().contains("reduce") }) {
            return byName
        }
        let fallbackProps: [String: Any] = [
            "FilterType": 1,
            "Name": "FLIP Reduce File Size",
            "FilterData": [
                "ColorSettings": [
                    "ImageSettings": [
                        "Compression Quality": 0.70,
                        "ImageCompression": "ImageJPEGCompress",
                        "ImageScaleSettings": [
                            "ImageResolution": 144,
                            "ImageScaleInterpolate": true,
                            "ImageSizeMax": 2400,
                            "ImageSizeMin": 0
                        ]
                    ]
                ]
            ]
        ]
        return QuartzFilter(properties: fallbackProps)
    }

    static func optimizePDF(inputPath: String, outputPath: String) throws -> [String: Any] {
        guard !FileManager.default.fileExists(atPath: outputPath),
              (try? FileManager.default.attributesOfItem(atPath: outputPath)) == nil else {
            throw BridgeError.operationFailed("Refusing to overwrite existing output PDF: \(outputPath)")
        }

        let inURL = URL(fileURLWithPath: inputPath)
        guard let doc = PDFDocument(url: inURL) else {
            throw BridgeError.operationFailed("Failed to open PDF: \(inputPath)")
        }
        if doc.isLocked {
            throw BridgeError.operationFailed("PDF is password-protected or encrypted")
        }
        let pageCount = doc.pageCount
        guard pageCount > 0 else {
            throw BridgeError.operationFailed("PDF has 0 pages")
        }
        if pageCount > 200 {
            throw BridgeError.operationFailed("PDF exceeds 200-page safety limit (\(pageCount) pages)")
        }

        let inputAttrs = try? FileManager.default.attributesOfItem(atPath: inputPath)
        let inputBytes = (inputAttrs?[.size] as? NSNumber)?.intValue ?? 0

        let outData = NSMutableData()
        guard let consumer = CGDataConsumer(data: outData as CFMutableData) else {
            throw BridgeError.operationFailed("Failed to allocate PDF data consumer")
        }
        var defaultBox = CGRect(x: 0, y: 0, width: 595, height: 842)
        guard let outCtx = CGContext(consumer: consumer, mediaBox: &defaultBox, nil) else {
            throw BridgeError.operationFailed("Failed to create PDF CGContext")
        }

        let filter = resolveReduceFileSizeFilter()
        let filterApplied = filter?.apply(to: outCtx) ?? false

        if filterApplied {
            for idx in 0..<pageCount {
                try autoreleasepool {
                    guard let page = doc.page(at: idx), let pageRef = page.pageRef else {
                        throw BridgeError.operationFailed("Failed to load PDF page \(idx + 1)")
                    }
                    let crop = page.bounds(for: .cropBox)
                    let tr = page.transform(for: .cropBox)
                    let db = crop.applying(tr)
                    var targetBox = CGRect(
                        origin: .zero,
                        size: CGSize(width: max(1, db.width), height: max(1, db.height))
                    )
                    let boxData = Data(bytes: &targetBox, count: MemoryLayout<CGRect>.size) as CFData
                    outCtx.beginPDFPage([kCGPDFContextMediaBox: boxData] as CFDictionary)
                    outCtx.saveGState()
                    outCtx.clip(to: targetBox)
                    let drawTransform = pageRef.getDrawingTransform(
                        .cropBox,
                        rect: targetBox,
                        rotate: 0,
                        preserveAspectRatio: true
                    )
                    outCtx.concatenate(drawTransform)
                    outCtx.drawPDFPage(pageRef)
                    outCtx.restoreGState()
                    outCtx.endPDFPage()
                }
            }
        } else {
            // Fallback: rasterize at 144 DPI JPEG if QuartzFilter is unavailable
            for idx in 0..<pageCount {
                try autoreleasepool {
                    guard let page = doc.page(at: idx) else {
                        throw BridgeError.operationFailed("Failed to load PDF page \(idx + 1)")
                    }
                    let crop = page.bounds(for: .cropBox)
                    let tr = page.transform(for: .cropBox)
                    let db = crop.applying(tr)
                    var targetBox = CGRect(
                        origin: .zero,
                        size: CGSize(width: max(1, db.width), height: max(1, db.height))
                    )
                    let cgImg = try renderPageToCGImage(page, dpi: 144.0, box: .cropBox)
                    let jpgData = NSMutableData()
                    guard let jpgDest = CGImageDestinationCreateWithData(
                        jpgData as CFMutableData,
                        UTType.jpeg.identifier as CFString,
                        1,
                        nil
                    ) else {
                        throw BridgeError.operationFailed("Failed to encode compressed PDF page image")
                    }
                    CGImageDestinationAddImage(
                        jpgDest,
                        cgImg,
                        [kCGImageDestinationLossyCompressionQuality: 0.72] as CFDictionary
                    )
                    guard CGImageDestinationFinalize(jpgDest),
                          let jpgSrc = CGImageSourceCreateWithData(jpgData as CFData, nil),
                          let jpgImg = CGImageSourceCreateImageAtIndex(jpgSrc, 0, nil) else {
                        throw BridgeError.operationFailed("Failed to decode compressed PDF page image")
                    }
                    let boxData = Data(bytes: &targetBox, count: MemoryLayout<CGRect>.size) as CFData
                    outCtx.beginPDFPage([kCGPDFContextMediaBox: boxData] as CFDictionary)
                    outCtx.draw(jpgImg, in: targetBox)
                    outCtx.endPDFPage()
                }
            }
        }

        outCtx.closePDF()

        let bytes = outData as Data
        guard !bytes.isEmpty else {
            throw BridgeError.operationFailed("PDF optimization produced empty output")
        }
        let outURL = URL(fileURLWithPath: outputPath)
        try bytes.write(to: outURL, options: .withoutOverwriting)

        return [
            "ok": true,
            "output": outputPath,
            "pages": pageCount,
            "inputBytes": inputBytes,
            "outputBytes": bytes.count,
            "quartzFilter": filterApplied
        ]
    }

    private static func sanitizeHTMLForOfflineImport(_ html: String) -> String {
        var out = html
        let stripBlockPatterns = [
            "<script\\b[^>]*>[\\s\\S]*?</script>",
            "<script\\b[^>]*>",
            "<iframe\\b[^>]*>[\\s\\S]*?</iframe>",
            "<iframe\\b[^>]*>",
            "<object\\b[^>]*>[\\s\\S]*?</object>",
            "<embed\\b[^>]*>",
            "<frame\\b[^>]*>",
            "<frameset\\b[^>]*>[\\s\\S]*?</frameset>",
            "<link\\b[^>]*>",
            "<base\\b[^>]*>",
            "<meta\\b[^>]*http-equiv\\s*=\\s*[\"']?refresh[^>]*>"
        ]
        for pattern in stripBlockPatterns {
            if let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
                out = regex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "")
            }
        }

        let srcDoubleQuoted = "(\\b(?:src|srcset|background|poster|data)\\s*=\\s*\")(?!\\s*data:image/)([^\"]*)(\")"
        let srcSingleQuoted = "(\\b(?:src|srcset|background|poster|data)\\s*=\\s*')(?!\\s*data:image/)([^']*)(')"
        let srcUnquoted = "(\\b(?:src|srcset|background|poster|data)\\s*=\\s*)(?!\"|'|\\s*data:image/)([^\\s>]+)"
        for (idx, pattern) in [srcDoubleQuoted, srcSingleQuoted, srcUnquoted].enumerated() {
            if let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
                let template = idx == 2 ? "$1\"#blocked\"" : "$1#blocked$3"
                out = regex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: template)
            }
        }

        let hrefFileDouble = "(\\bhref\\s*=\\s*\")\\s*file:[^\"]*(\")"
        let hrefFileSingle = "(\\bhref\\s*=\\s*')\\s*file:[^']*(')"
        let hrefFileUnquoted = "(\\bhref\\s*=\\s*)file:[^\\s>]+"
        for (idx, pattern) in [hrefFileDouble, hrefFileSingle, hrefFileUnquoted].enumerated() {
            if let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
                let template = idx == 2 ? "$1\"#blocked\"" : "$1#blocked$2"
                out = regex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: template)
            }
        }

        // Stop before '<' so an @import without trailing semicolon never swallows </style> and the HTML body.
        if let cssImportRegex = try? NSRegularExpression(pattern: "@import\\b[^;<\\n]*[;\\n]?", options: [.caseInsensitive]) {
            out = cssImportRegex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "")
        }
        if let cssUrlRegex = try? NSRegularExpression(pattern: "url\\(\\s*(?![\"']?data:image/)[^)]*\\)", options: [.caseInsensitive]) {
            out = cssUrlRegex.stringByReplacingMatches(in: out, range: NSRange(out.startIndex..., in: out), withTemplate: "url()")
        }
        return out
    }

    private static func withAquaAppearance<T>(_ body: () throws -> T) throws -> T {
        guard let aqua = NSAppearance(named: .aqua) else {
            return try body()
        }
        var result: Result<T, Error>!
        aqua.performAsCurrentDrawingAppearance {
            do {
                result = .success(try body())
            } catch {
                result = .failure(error)
            }
        }
        return try result.get()
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

        let attrString: NSAttributedString = try withAquaAppearance {
            do {
                if inExt == "html" || inExt == "htm" {
                    let rawData = try Data(contentsOf: inURL)
                    if rawData.count > maxHtmlInputBytes {
                        throw BridgeError.operationFailed("HTML document exceeds 20 MiB safety limit")
                    }
                    let rawStr: String
                    if let utf8 = String(data: rawData, encoding: .utf8) {
                        rawStr = utf8
                    } else if let latin1 = String(data: rawData, encoding: .isoLatin1) {
                        rawStr = latin1
                    } else {
                        throw BridgeError.operationFailed("Could not decode HTML text encoding")
                    }
                    let sanitized = sanitizeHTMLForOfflineImport(rawStr)
                    let docOpts: [NSAttributedString.DocumentReadingOptionKey: Any] = [
                        .documentType: NSAttributedString.DocumentType.html,
                        .characterEncoding: String.Encoding.utf8.rawValue
                    ]
                    return try NSAttributedString(data: Data(sanitized.utf8), options: docOpts, documentAttributes: nil)
                } else {
                    var docOpts: [NSAttributedString.DocumentReadingOptionKey: Any] = [:]
                    if inExt == "txt" || inExt == "md" || inExt == "markdown" {
                        docOpts[.documentType] = NSAttributedString.DocumentType.plain
                        docOpts[.characterEncoding] = String.Encoding.utf8.rawValue
                    }
                    return try NSAttributedString(url: inURL, options: docOpts, documentAttributes: nil)
                }
            } catch let bridgeErr as BridgeError {
                throw bridgeErr
            } catch {
                throw BridgeError.operationFailed("Failed to parse document \(inputPath): \(error.localizedDescription)")
            }
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
        let contentSize = CGSize(width: size.width - margin * 2, height: size.height - margin * 2)

        let mutable = NSMutableAttributedString(attributedString: attr)
        let fullRange = NSRange(location: 0, length: mutable.length)
        try withAquaAppearance {
            var fontUpdates: [(NSRange, NSFont)] = []
            mutable.enumerateAttribute(.font, in: fullRange) { val, range, _ in
                if let f = val as? NSFont {
                    if f.pointSize > 240 {
                        let clamped = NSFont(descriptor: f.fontDescriptor, size: 240) ?? NSFont.systemFont(ofSize: 240)
                        fontUpdates.append((range, clamped))
                    }
                } else {
                    fontUpdates.append((range, NSFont.systemFont(ofSize: 11.5)))
                }
            }
            for (range, font) in fontUpdates {
                mutable.addAttribute(.font, value: font, range: range)
            }

            var colorUpdates: [(NSRange, NSColor)] = []
            mutable.enumerateAttribute(.foregroundColor, in: fullRange) { val, range, _ in
                if val == nil {
                    colorUpdates.append((range, NSColor.black))
                } else if let c = val as? NSColor, let resolved = c.usingColorSpace(.sRGB) {
                    colorUpdates.append((range, resolved))
                }
            }
            for (range, color) in colorUpdates {
                mutable.addAttribute(.foregroundColor, value: color, range: range)
            }

            mutable.enumerateAttribute(.attachment, in: fullRange) { val, _, _ in
                guard let att = val as? NSTextAttachment else { return }
                let img = att.image ?? att.fileWrapper?.regularFileContents.flatMap { NSImage(data: $0) }
                guard let nsImg = img else { return }
                let sz = att.bounds.size != .zero ? att.bounds.size : nsImg.size
                if sz.width > contentSize.width || sz.height > contentSize.height {
                    let scale = min(contentSize.width / max(1, sz.width), contentSize.height / max(1, sz.height))
                    let fitted = CGSize(width: floor(sz.width * scale), height: floor(sz.height * scale))
                    nsImg.size = fitted
                    att.image = nsImg
                    att.attachmentCell = NSTextAttachmentCell(imageCell: nsImg)
                    att.bounds = CGRect(origin: .zero, size: fitted)
                }
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

        let storage = NSTextStorage(attributedString: mutable)
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)

        let totalGlyphs = layoutManager.numberOfGlyphs
        var glyphIndex = 0
        var pageCount = 0

        try withAquaAppearance {
            while glyphIndex < totalGlyphs || pageCount == 0 {
                pageCount += 1
                if pageCount > 200 {
                    throw BridgeError.operationFailed("PDF output exceeds 200-page safety limit")
                }

                let container = NSTextContainer(size: contentSize)
                container.lineFragmentPadding = 0
                layoutManager.addTextContainer(container)
                let glyphRange = layoutManager.glyphRange(for: container)

                if glyphRange.length == 0 && pageCount > 1 {
                    layoutManager.removeTextContainer(at: layoutManager.textContainers.count - 1)
                    pageCount -= 1
                    break
                }

                pdfCtx.beginPDFPage(nil)
                pdfCtx.setFillColor(CGColor.white)
                pdfCtx.fill(mediaBox)

                if glyphRange.length > 0 {
                    pdfCtx.saveGState()
                    pdfCtx.translateBy(x: 0, y: size.height)
                    pdfCtx.scaleBy(x: 1, y: -1)
                    let nsCtx = NSGraphicsContext(cgContext: pdfCtx, flipped: true)
                    NSGraphicsContext.saveGraphicsState()
                    NSGraphicsContext.current = nsCtx
                    let drawOrigin = CGPoint(x: margin, y: margin)
                    layoutManager.drawBackground(forGlyphRange: glyphRange, at: drawOrigin)
                    layoutManager.drawGlyphs(forGlyphRange: glyphRange, at: drawOrigin)
                    NSGraphicsContext.restoreGraphicsState()
                    pdfCtx.restoreGState()
                }

                pdfCtx.endPDFPage()

                if glyphRange.length == 0 {
                    break
                }
                glyphIndex = NSMaxRange(glyphRange)
                if glyphIndex >= totalGlyphs {
                    break
                }
            }
        }

        pdfCtx.closePDF()

        let bytes = pdfData as Data
        guard !bytes.isEmpty else {
            throw BridgeError.operationFailed("PDF rendering produced empty output")
        }
        try bytes.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        return ["ok": true, "output": outputPath, "pages": pageCount]
    }

    private static func runProcessWithLimit(
        executablePath: String,
        arguments: [String],
        timeoutSeconds: TimeInterval,
        maxOutputBytes: Int
    ) throws -> (exitCode: Int32, data: Data) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: executablePath)
        proc.arguments = arguments
        let outPipe = Pipe()
        proc.standardOutput = outPipe
        proc.standardError = FileHandle.nullDevice

        var outputData = Data()
        var exceededLimit = false
        let readLock = NSLock()
        let readGroup = DispatchGroup()
        readGroup.enter()

        let readHandle = outPipe.fileHandleForReading
        DispatchQueue.global(qos: .userInitiated).async {
            defer { readGroup.leave() }
            while true {
                let chunk = readHandle.availableData
                if chunk.isEmpty {
                    break
                }
                readLock.lock()
                if outputData.count + chunk.count > maxOutputBytes {
                    exceededLimit = true
                    readLock.unlock()
                    if proc.isRunning {
                        proc.terminate()
                    }
                    break
                }
                outputData.append(chunk)
                readLock.unlock()
            }
        }

        let exitSem = DispatchSemaphore(value: 0)
        proc.terminationHandler = { _ in
            exitSem.signal()
        }

        try proc.run()

        if exitSem.wait(timeout: .now() + timeoutSeconds) == .timedOut {
            if proc.isRunning {
                proc.terminate()
            }
            _ = exitSem.wait(timeout: .now() + 1.0)
            if proc.isRunning {
                kill(proc.processIdentifier, SIGKILL)
            }
            _ = readGroup.wait(timeout: .now() + 1.0)
            throw BridgeError.operationFailed("\(executablePath) timed out after \(Int(timeoutSeconds))s")
        }

        _ = readGroup.wait(timeout: .now() + 2.0)
        readLock.lock()
        let over = exceededLimit
        let finalData = outputData
        readLock.unlock()
        if over {
            throw BridgeError.operationFailed("Extracted entry exceeds 256 MiB safety limit")
        }
        return (proc.terminationStatus, finalData)
    }

    private static func verifyZipPreviewWithinLimit(_ zipPath: String) throws {
        if let attrs = try? FileManager.default.attributesOfItem(atPath: zipPath),
           let ftype = attrs[.type] as? FileAttributeType,
           ftype == .typeSymbolicLink {
            throw BridgeError.operationFailed("Refusing symlinked iWork archive")
        }
        let (code, listData) = try runProcessWithLimit(
            executablePath: "/usr/bin/unzip",
            arguments: ["-l", zipPath],
            timeoutSeconds: 10.0,
            maxOutputBytes: 4 * 1024 * 1024
        )
        guard code == 0, let listing = String(data: listData, encoding: .utf8) else {
            throw BridgeError.operationFailed("Failed to inspect iWork archive")
        }
        var totalDeclared: Int64 = 0
        for line in listing.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let parts = trimmed.split(whereSeparator: { $0.isWhitespace })
            guard parts.count >= 4, let sizeVal = Int64(parts[0]) else { continue }
            totalDeclared = totalDeclared.addingReportingOverflow(sizeVal).partialValue
            if sizeVal > Int64(maxContainerBytes) || totalDeclared > Int64(maxContainerBytes) {
                throw BridgeError.operationFailed("iWork container exceeds 256 MiB safety limit")
            }
        }
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

        guard !FileManager.default.fileExists(atPath: outputPath),
              (try? FileManager.default.attributesOfItem(atPath: outputPath)) == nil else {
            throw BridgeError.operationFailed("Refusing to overwrite existing output: \(outputPath)")
        }

        if let inAttrs = try? FileManager.default.attributesOfItem(atPath: inputPath),
           let inType = inAttrs[.type] as? FileAttributeType,
           inType == .typeSymbolicLink {
            throw BridgeError.operationFailed("Refusing symlinked iWork input")
        }

        var isDir: ObjCBool = false
        let inputExists = FileManager.default.fileExists(atPath: inputPath, isDirectory: &isDir)
        guard inputExists else {
            throw BridgeError.operationFailed("Input iWork file does not exist: \(inputPath)")
        }
        if !isDir.boolValue {
            try verifyZipPreviewWithinLimit(inputPath)
        }

        let bundleId = "com.apple.iWork.\(appName)"
        if NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) != nil {
            let escapedIn = inputPath
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            let escapedOut = outputPath
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            let script = """
            with timeout of 25 seconds
                tell application "\(appName)"
                    set doc to open POSIX file "\(escapedIn)"
                    export doc to POSIX file "\(escapedOut)" as \(appleFormat)
                    close doc saving no
                end tell
            end timeout
            """
            let (_, err) = FinderBridge.runAppleScript(script, timeoutSeconds: 26.0)
            if err == nil && FileManager.default.fileExists(atPath: outputPath) {
                return ["ok": true, "engine": appName, "output": outputPath]
            }
        }

        // Fallback: if format is PDF and QuickLook/Preview.pdf exists inside iWork package/zip
        if format == "PDF" {
            if isDir.boolValue {
                let previewInsidePkg = (inputPath as NSString).appendingPathComponent("QuickLook/Preview.pdf")
                if FileManager.default.fileExists(atPath: previewInsidePkg) {
                    let attrs = try FileManager.default.attributesOfItem(atPath: previewInsidePkg)
                    if let ftype = attrs[.type] as? FileAttributeType, ftype == .typeSymbolicLink {
                        throw BridgeError.operationFailed("Refusing symlinked iWork Preview.pdf")
                    }
                    let size = (attrs[.size] as? NSNumber)?.intValue ?? 0
                    if size <= 0 || size > maxContainerBytes {
                        throw BridgeError.operationFailed("iWork Preview.pdf exceeds 256 MiB safety limit")
                    }
                    let data = try Data(contentsOf: URL(fileURLWithPath: previewInsidePkg))
                    guard data.starts(with: Data("%PDF-".utf8)) else {
                        throw BridgeError.operationFailed("iWork Preview.pdf is not a valid PDF")
                    }
                    try data.write(to: URL(fileURLWithPath: outputPath), options: .withoutOverwriting)
                    return ["ok": true, "engine": "iwork-preview", "output": outputPath]
                }
            } else {
                let (code, data) = try runProcessWithLimit(
                    executablePath: "/usr/bin/unzip",
                    arguments: ["-p", inputPath, "QuickLook/Preview.pdf"],
                    timeoutSeconds: 15.0,
                    maxOutputBytes: maxContainerBytes
                )
                if code == 0 && !data.isEmpty && data.starts(with: Data("%PDF-".utf8)) {
                    try data.write(to: URL(fileURLWithPath: outputPath), options: .withoutOverwriting)
                    return ["ok": true, "engine": "iwork-preview", "output": outputPath]
                }
            }
        }

        throw BridgeError.operationFailed("\(appName) is not installed or failed to export \(inExt) to \(format)")
    }

    static func createIWorkDocument(kind: String, outputPath: String) throws -> [String: Any] {
        let appName: String
        switch kind.lowercased() {
        case "pages": appName = "Pages"
        case "numbers": appName = "Numbers"
        case "key", "keynote": appName = "Keynote"
        default: throw BridgeError.invalidArguments("Unknown iWork kind: \(kind)")
        }

        let bundleId = "com.apple.iWork.\(appName)"
        guard NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleId) != nil else {
            throw BridgeError.operationFailed("\(appName).app is not installed")
        }

        let escapedOut = outputPath
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let script = """
        with timeout of 25 seconds
            tell application "\(appName)"
                set newDoc to make new document
                save newDoc in POSIX file "\(escapedOut)"
                close newDoc saving no
            end tell
        end timeout
        """
        let (_, err) = FinderBridge.runAppleScript(script, timeoutSeconds: 26.0)
        if let err = err {
            throw BridgeError.operationFailed("\(appName) failed to create document: \(err)")
        }
        guard FileManager.default.fileExists(atPath: outputPath) else {
            throw BridgeError.operationFailed("\(appName) did not produce output at \(outputPath)")
        }
        return ["ok": true, "engine": appName, "output": outputPath]
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

    static func convertImage(
        inputPath: String,
        format: String,
        outputPath: String,
        quality: Double = 0.92,
        preserveExif: Bool = true,
        stripGps: Bool = false
    ) throws -> [String: Any] {
        let (rawImage, orientation, srcProps) = try decodeCGImageAndProps(path: inputPath)

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
        let nonRgbSpace = rawImage.colorSpace?.model != .rgb
        var finalImage: CGImage
        if orientation != 1 || flattenAlpha || nonRgbSpace {
            finalImage = normalizeOrientedImage(rawImage, orientation: orientation, flattenAlpha: flattenAlpha) ?? rawImage
        } else {
            finalImage = rawImage
        }

        if fmtUpper == "ICNS" {
            let maxDim = max(finalImage.width, finalImage.height)
            let validSizes = [16, 32, 64, 128, 256, 512]
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

        var props: [CFString: Any] = [:]
        if preserveExif, let srcProps = srcProps {
            let metadataKeys: [CFString] = [
                kCGImagePropertyExifDictionary,
                kCGImagePropertyExifAuxDictionary,
                kCGImagePropertyTIFFDictionary,
                kCGImagePropertyIPTCDictionary,
                kCGImagePropertyGPSDictionary,
            ]
            for key in metadataKeys {
                if key == kCGImagePropertyGPSDictionary && stripGps {
                    continue
                }
                if let dict = srcProps[key] {
                    if key == kCGImagePropertyTIFFDictionary, var tiffDict = dict as? [CFString: Any] {
                        tiffDict[kCGImagePropertyTIFFOrientation] = 1
                        props[key] = tiffDict
                    } else {
                        props[key] = dict
                    }
                }
            }
        }

        props[kCGImagePropertyOrientation] = 1
        if fmtUpper == "JPG" || fmtUpper == "JPEG" || fmtUpper == "HEIC" {
            props[kCGImageDestinationLossyCompressionQuality] = quality
        }
        CGImageDestinationAddImage(dest, finalImage, props as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw BridgeError.operationFailed("ImageIO failed to write \(outputPath)")
        }
        return ["ok": true, "output": outputPath, "width": finalImage.width, "height": finalImage.height]
    }

    static func removeBackground(inputPath: String, outputPath: String) throws -> [String: Any] {
        guard !FileManager.default.fileExists(atPath: outputPath),
              (try? FileManager.default.attributesOfItem(atPath: outputPath)) == nil else {
            throw BridgeError.operationFailed("Refusing to overwrite existing output PNG: \(outputPath)")
        }

        let (rawImage, orientation, _) = try decodeCGImageAndProps(path: inputPath)

        let uprightImage: CGImage
        if orientation != 1 || rawImage.colorSpace?.model != .rgb {
            uprightImage = normalizeOrientedImage(rawImage, orientation: orientation, flattenAlpha: false) ?? rawImage
        } else {
            uprightImage = rawImage
        }

        guard #available(macOS 14.0, *) else {
            throw BridgeError.operationFailed("Vision background removal requires macOS 14.0 or newer")
        }

        let request = VNGenerateForegroundInstanceMaskRequest()
        let handler = VNImageRequestHandler(cgImage: uprightImage, options: [:])
        try handler.perform([request])

        guard let observation = request.results?.first, !observation.allInstances.isEmpty else {
            throw BridgeError.operationFailed("No foreground subject detected in image")
        }

        let maskPixelBuffer = try observation.generateScaledMaskForImage(
            forInstances: observation.allInstances,
            from: handler
        )

        let inputCI = CIImage(cgImage: uprightImage)
        var maskCI = CIImage(cvPixelBuffer: maskPixelBuffer)
        if maskCI.extent.size != inputCI.extent.size || maskCI.extent.origin != inputCI.extent.origin {
            let sx = inputCI.extent.width / max(1.0, maskCI.extent.width)
            let sy = inputCI.extent.height / max(1.0, maskCI.extent.height)
            maskCI = maskCI
                .transformed(by: CGAffineTransform(translationX: -maskCI.extent.origin.x, y: -maskCI.extent.origin.y))
                .transformed(by: CGAffineTransform(scaleX: sx, y: sy))
                .transformed(by: CGAffineTransform(translationX: inputCI.extent.origin.x, y: inputCI.extent.origin.y))
        }

        let transparentBg = CIImage(color: .clear).cropped(to: inputCI.extent)
        guard let blendFilter = CIFilter(name: "CIBlendWithMask", parameters: [
            kCIInputImageKey: inputCI,
            kCIInputBackgroundImageKey: transparentBg,
            kCIInputMaskImageKey: maskCI
        ]), let outputCI = blendFilter.outputImage else {
            throw BridgeError.operationFailed("CoreImage CIBlendWithMask failed to composite foreground")
        }

        guard let srgb = CGColorSpace(name: CGColorSpace.sRGB) else {
            throw BridgeError.operationFailed("Failed to allocate sRGB color space")
        }
        let ciCtx = CIContext(options: [.workingColorSpace: srgb])
        guard let outCG = ciCtx.createCGImage(outputCI, from: inputCI.extent, format: .RGBA8, colorSpace: srgb) else {
            throw BridgeError.operationFailed("CoreImage failed to render cutout PNG")
        }

        let outURL = URL(fileURLWithPath: outputPath)
        guard let dest = CGImageDestinationCreateWithURL(outURL as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            throw BridgeError.operationFailed("Failed to create ImageIO PNG destination")
        }
        let outProps: [CFString: Any] = [kCGImagePropertyOrientation: 1]
        CGImageDestinationAddImage(dest, outCG, outProps as CFDictionary)
        guard CGImageDestinationFinalize(dest) else {
            throw BridgeError.operationFailed("ImageIO failed to finalize cutout PNG: \(outputPath)")
        }

        return [
            "ok": true,
            "output": outputPath,
            "width": outCG.width,
            "height": outCG.height,
            "instances": observation.allInstances.count
        ]
    }

    // MARK: - AVFoundation Native Media Export
    private static func convertVideoToGIF(inURL: URL, outURL: URL) throws -> [String: Any] {
        let asset = AVURLAsset(url: inURL)
        let sem = DispatchSemaphore(value: 0)
        var gifError: Error?
        var totalFramesWritten = 0

        Task {
            do {
                let durationCM = try await asset.load(.duration)
                let durationSecs = max(0.2, min(15.0, CMTimeGetSeconds(durationCM)))
                let fps = 12.0
                let frameCount = max(1, min(180, Int((durationSecs * fps).rounded())))
                let frameDelay = 1.0 / fps

                let generator = AVAssetImageGenerator(asset: asset)
                generator.appliesPreferredTrackTransform = true
                generator.maximumSize = CGSize(width: 480, height: 480)
                generator.requestedTimeToleranceBefore = CMTime(seconds: 0.05, preferredTimescale: 600)
                generator.requestedTimeToleranceAfter = CMTime(seconds: 0.05, preferredTimescale: 600)

                var frames: [CGImage] = []
                frames.reserveCapacity(frameCount)
                for idx in 0..<frameCount {
                    let t = CMTime(seconds: Double(idx) * frameDelay, preferredTimescale: 600)
                    if let result = try? await generator.image(at: t) {
                        frames.append(result.image)
                    }
                }
                guard !frames.isEmpty else {
                    throw BridgeError.operationFailed("Video produced zero frames for animated GIF export")
                }

                guard let dest = CGImageDestinationCreateWithURL(
                    outURL as CFURL,
                    UTType.gif.identifier as CFString,
                    frames.count,
                    nil
                ) else {
                    throw BridgeError.operationFailed("Failed to create animated GIF destination")
                }

                let fileProps: [CFString: Any] = [
                    kCGImagePropertyGIFDictionary: [
                        kCGImagePropertyGIFLoopCount: 0
                    ]
                ]
                CGImageDestinationSetProperties(dest, fileProps as CFDictionary)

                let frameProps: [CFString: Any] = [
                    kCGImagePropertyGIFDictionary: [
                        kCGImagePropertyGIFDelayTime: frameDelay
                    ]
                ]
                for img in frames {
                    CGImageDestinationAddImage(dest, img, frameProps as CFDictionary)
                }
                guard CGImageDestinationFinalize(dest) else {
                    throw BridgeError.operationFailed("Failed to finalize animated GIF")
                }
                totalFramesWritten = frames.count
            } catch {
                gifError = error
            }
            sem.signal()
        }
        sem.wait()

        if let err = gifError {
            throw BridgeError.operationFailed(err.localizedDescription)
        }
        return ["ok": true, "output": outURL.path, "frames": totalFramesWritten]
    }

    private static func convertAudioViaExtAudioFile(inURL: URL, format: String, outURL: URL) throws -> [String: Any] {
        var inExt: ExtAudioFileRef?
        let openStatus = ExtAudioFileOpenURL(inURL as CFURL, &inExt)
        guard openStatus == noErr, let inRef = inExt else {
            throw BridgeError.operationFailed("Could not open audio file via ExtAudioFile (\(openStatus)): \(inURL.path)")
        }
        defer { ExtAudioFileDispose(inRef) }

        var srcDesc = AudioStreamBasicDescription()
        var asbdSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        let fmtStatus = ExtAudioFileGetProperty(inRef, kExtAudioFileProperty_FileDataFormat, &asbdSize, &srcDesc)
        guard fmtStatus == noErr else {
            throw BridgeError.operationFailed("Could not inspect audio format (\(fmtStatus)): \(inURL.path)")
        }

        let sampleRate = srcDesc.mSampleRate > 0 ? srcDesc.mSampleRate : 44100.0
        let channels = max(1, srcDesc.mChannelsPerFrame)
        let bytesPerFrame = 2 * channels
        var clientDesc = AudioStreamBasicDescription(
            mSampleRate: sampleRate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
            mBytesPerPacket: bytesPerFrame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytesPerFrame,
            mChannelsPerFrame: channels,
            mBitsPerChannel: 16,
            mReserved: 0
        )

        let setInClientStatus = ExtAudioFileSetProperty(inRef, kExtAudioFileProperty_ClientDataFormat, asbdSize, &clientDesc)
        guard setInClientStatus == noErr else {
            throw BridgeError.operationFailed("Failed to configure PCM decoder format (\(setInClientStatus))")
        }

        // Buffer up to 4608 frames before opening output so short (<4608 frame) inputs choose a matching FLAC block size.
        let prefillCap: UInt32 = 4608
        let prefillBytes = Int(prefillCap * bytesPerFrame)
        let prefillBuf = UnsafeMutableRawPointer.allocate(byteCount: prefillBytes, alignment: 4)
        defer { prefillBuf.deallocate() }
        prefillBuf.initializeMemory(as: UInt8.self, repeating: 0, count: prefillBytes)

        var prefillFrames: UInt32 = 0
        while prefillFrames < prefillCap {
            let remaining = prefillCap - prefillFrames
            let offsetPtr = prefillBuf.advanced(by: Int(prefillFrames * bytesPerFrame))
            var nFrames = remaining
            var abl = AudioBufferList(
                mNumberBuffers: 1,
                mBuffers: AudioBuffer(
                    mNumberChannels: channels,
                    mDataByteSize: remaining * bytesPerFrame,
                    mData: offsetPtr
                )
            )
            let readErr = ExtAudioFileRead(inRef, &nFrames, &abl)
            if readErr != noErr {
                throw BridgeError.operationFailed("Failed reading audio frames (\(readErr))")
            }
            if nFrames == 0 {
                break
            }
            prefillFrames += nFrames
        }

        guard prefillFrames > 0 else {
            throw BridgeError.operationFailed("Audio file contains no decodable frames: \(inURL.path)")
        }

        let fmtUpper = format.uppercased()
        let fileTypeID: AudioFileTypeID
        var dstDesc: AudioStreamBasicDescription
        var initialFramesToWrite = prefillFrames

        switch fmtUpper {
        case "FLAC":
            fileTypeID = kAudioFileFLACType
            let blockSize: UInt32
            if prefillFrames >= 4608 {
                blockSize = 4608
            } else if prefillFrames >= 1152 {
                blockSize = 1152
            } else if prefillFrames >= 576 {
                blockSize = 576
            } else {
                blockSize = 192
                if prefillFrames < 192 {
                    initialFramesToWrite = 192
                }
            }
            dstDesc = AudioStreamBasicDescription(
                mSampleRate: sampleRate,
                mFormatID: kAudioFormatFLAC,
                mFormatFlags: 0,
                mBytesPerPacket: 0,
                mFramesPerPacket: blockSize,
                mBytesPerFrame: 0,
                mChannelsPerFrame: channels,
                mBitsPerChannel: 0,
                mReserved: 0
            )
        case "CAF":
            fileTypeID = kAudioFileCAFType
            dstDesc = clientDesc
        default:
            throw BridgeError.invalidArguments("ExtAudioFile audio export does not support \(format)")
        }

        var outExt: ExtAudioFileRef?
        let createStatus = ExtAudioFileCreateWithURL(
            outURL as CFURL,
            fileTypeID,
            &dstDesc,
            nil,
            AudioFileFlags.eraseFile.rawValue,
            &outExt
        )
        guard createStatus == noErr, let outRef = outExt else {
            throw BridgeError.operationFailed("Failed creating \(fmtUpper) audio output (\(createStatus)): \(outURL.path)")
        }

        var disposed = false
        defer {
            if !disposed {
                ExtAudioFileDispose(outRef)
            }
        }

        let setOutClientStatus = ExtAudioFileSetProperty(outRef, kExtAudioFileProperty_ClientDataFormat, asbdSize, &clientDesc)
        guard setOutClientStatus == noErr else {
            try? FileManager.default.removeItem(at: outURL)
            throw BridgeError.operationFailed("Failed to configure \(fmtUpper) encoder format (\(setOutClientStatus))")
        }

        var initialABL = AudioBufferList(
            mNumberBuffers: 1,
            mBuffers: AudioBuffer(
                mNumberChannels: channels,
                mDataByteSize: initialFramesToWrite * bytesPerFrame,
                mData: prefillBuf
            )
        )
        let firstWriteErr = ExtAudioFileWrite(outRef, initialFramesToWrite, &initialABL)
        guard firstWriteErr == noErr else {
            try? FileManager.default.removeItem(at: outURL)
            throw BridgeError.operationFailed("Failed writing \(fmtUpper) initial frames (\(firstWriteErr))")
        }

        var totalWritten: UInt64 = UInt64(initialFramesToWrite)
        if prefillFrames == prefillCap {
            let chunkCap: UInt32 = 4096
            let chunkBytes = Int(chunkCap * bytesPerFrame)
            let chunkBuf = UnsafeMutableRawPointer.allocate(byteCount: chunkBytes, alignment: 4)
            defer { chunkBuf.deallocate() }

            while true {
                var nFrames = chunkCap
                var abl = AudioBufferList(
                    mNumberBuffers: 1,
                    mBuffers: AudioBuffer(
                        mNumberChannels: channels,
                        mDataByteSize: UInt32(chunkBytes),
                        mData: chunkBuf
                    )
                )
                let readErr = ExtAudioFileRead(inRef, &nFrames, &abl)
                if readErr != noErr {
                    try? FileManager.default.removeItem(at: outURL)
                    throw BridgeError.operationFailed("Failed reading audio stream (\(readErr))")
                }
                if nFrames == 0 {
                    break
                }
                let writeErr = ExtAudioFileWrite(outRef, nFrames, &abl)
                if writeErr != noErr {
                    try? FileManager.default.removeItem(at: outURL)
                    throw BridgeError.operationFailed("Failed writing \(fmtUpper) frames (\(writeErr))")
                }
                totalWritten += UInt64(nFrames)
            }
        }

        disposed = true
        let disposeStatus = ExtAudioFileDispose(outRef)
        guard disposeStatus == noErr else {
            try? FileManager.default.removeItem(at: outURL)
            throw BridgeError.operationFailed("Failed finalizing \(fmtUpper) output (\(disposeStatus))")
        }

        let attrs = try? FileManager.default.attributesOfItem(atPath: outURL.path)
        let outSize = (attrs?[.size] as? NSNumber)?.intValue ?? 0
        let minSize = fmtUpper == "FLAC" ? 43 : 1
        guard outSize >= minSize else {
            try? FileManager.default.removeItem(at: outURL)
            throw BridgeError.operationFailed("\(fmtUpper) encoding produced incomplete output (\(outSize) bytes)")
        }

        return [
            "ok": true,
            "output": outURL.path,
            "frames": totalWritten,
            "engine": "ExtAudioFile"
        ]
    }

    static func convertAV(inputPath: String, format: String, outputPath: String) throws -> [String: Any] {
        let inURL = URL(fileURLWithPath: inputPath)
        let outURL = URL(fileURLWithPath: outputPath)
        let fmtUpper = format.uppercased()

        if fmtUpper == "GIF" {
            return try convertVideoToGIF(inURL: inURL, outURL: outURL)
        }
        if fmtUpper == "FLAC" || fmtUpper == "CAF" {
            return try convertAudioViaExtAudioFile(inURL: inURL, format: fmtUpper, outURL: outURL)
        }
        if fmtUpper == "M4A" && inURL.pathExtension.lowercased() == "alac" {
            return try convertM4AViaAfconvert(inURL: inURL, outURL: outURL)
        }

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

    private static func convertM4AViaAfconvert(inURL: URL, outURL: URL) throws -> [String: Any] {
        let afconvertPath = "/usr/bin/afconvert"
        guard FileManager.default.isExecutableFile(atPath: afconvertPath) else {
            throw BridgeError.operationFailed("afconvert not found for ALAC -> M4A conversion")
        }
        try? FileManager.default.removeItem(at: outURL)
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: afconvertPath)
        proc.arguments = ["-f", "m4af", "-d", "aac", inURL.path, outURL.path]
        let errPipe = Pipe()
        proc.standardError = errPipe
        proc.standardOutput = Pipe()
        try proc.run()
        proc.waitUntilExit()
        guard proc.terminationStatus == 0 else {
            try? FileManager.default.removeItem(at: outURL)
            let errText = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw BridgeError.operationFailed(errText.isEmpty ? "afconvert exited with status \(proc.terminationStatus)" : errText)
        }
        return ["ok": true, "output": outURL.path, "engine": "afconvert"]
    }
}
