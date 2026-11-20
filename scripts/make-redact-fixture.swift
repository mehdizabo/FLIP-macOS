#!/usr/bin/env swift
// Generates the fake-data redaction fixtures used by the REDACT tool tests.
// Usage: swift scripts/make-redact-fixture.swift <out-dir>
//
// Outputs:
//   redact-sample.png        1200x800 white image, black text (fake values only)
//   redact-sample-rot6.jpg   same picture stored rotated 90° CCW, EXIF orientation 6 (shows upright)
//   redact-sample.expected.json  normalised top-left boxes of the sensitive values
//
// All values are fake; none of them belongs to a real person.

import Foundation
import CoreGraphics
import CoreText
import ImageIO
import UniformTypeIdentifiers

let imageWidth = 1200
let imageHeight = 800
let margin: CGFloat = 80
let fontSize: CGFloat = 40
let linePitch: CGFloat = 96
let topMargin: CGFloat = 50

struct FixtureLine {
    let text: String
    let kind: String?
    let value: String?
}

let lines: [FixtureLine] = [
    FixtureLine(text: "IBAN IT60 X054 2811 1010 0000 0123 456", kind: "iban", value: "IT60 X054 2811 1010 0000 0123 456"),
    FixtureLine(text: "Card 4111 1111 1111 1111", kind: "card", value: "4111 1111 1111 1111"),
    FixtureLine(text: "Mail mario.rossi@example.com", kind: "email", value: "mario.rossi@example.com"),
    FixtureLine(text: "Tel +39 333 1234567", kind: "phone", value: "+39 333 1234567"),
    FixtureLine(text: "CF RSSMRA85T10A562S", kind: "taxcode", value: "RSSMRA85T10A562S"),
    FixtureLine(text: "P.IVA 12345678903", kind: "vat", value: "12345678903"),
    FixtureLine(text: "Invoice 12/03/2025 total 1.234,56 EUR", kind: nil, value: nil),
]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func round6(_ value: CGFloat) -> Double {
    (Double(value) * 1_000_000).rounded() / 1_000_000
}

guard CommandLine.arguments.count >= 2 else {
    fail("usage: swift scripts/make-redact-fixture.swift <out-dir>")
}
let outDir = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB),
      let ctx = CGContext(
        data: nil,
        width: imageWidth,
        height: imageHeight,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
      ) else {
    fail("could not create the drawing context")
}

ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
ctx.fill(CGRect(x: 0, y: 0, width: imageWidth, height: imageHeight))
ctx.setShouldSmoothFonts(false)

let font = CTFontCreateWithName("HelveticaNeue" as CFString, fontSize, nil)
let attributes: [NSAttributedString.Key: Any] = [
    NSAttributedString.Key(kCTFontAttributeName as String): font,
    NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(red: 0, green: 0, blue: 0, alpha: 1),
]

var expected: [[String: Any]] = []

for (index, item) in lines.enumerated() {
    let attr = NSAttributedString(string: item.text, attributes: attributes)
    let line = CTLineCreateWithAttributedString(attr)

    var ascent: CGFloat = 0
    var descent: CGFloat = 0
    var leading: CGFloat = 0
    _ = CTLineGetTypographicBounds(line, &ascent, &descent, &leading)

    let lineTop = topMargin + CGFloat(index) * linePitch
    let baseline = CGFloat(imageHeight) - lineTop - ascent

    ctx.textPosition = CGPoint(x: margin, y: baseline)
    CTLineDraw(line, ctx)

    guard let kind = item.kind, let value = item.value else { continue }

    let utf16 = Array(item.text.utf16)
    let valueUTF16 = Array(value.utf16)
    guard let start = utf16.firstRange(of: valueUTF16)?.lowerBound else {
        fail("value not found in line: \(value)")
    }
    let end = start + valueUTF16.count
    let startOffset = CTLineGetOffsetForStringIndex(line, CFIndex(start), nil)
    let endOffset = CTLineGetOffsetForStringIndex(line, CFIndex(end), nil)

    // Vertical extent of the value's own glyph ink (relative to the baseline).
    let valueLine = CTLineCreateWithAttributedString(NSAttributedString(string: value, attributes: attributes))
    let ink = CTLineGetBoundsWithOptions(valueLine, .useGlyphPathBounds)

    let x0 = margin + startOffset
    let x1 = margin + endOffset
    let inkTop = baseline + ink.maxY
    let inkBottom = baseline + ink.minY
    let box = [
        round6(x0 / CGFloat(imageWidth)),
        round6((CGFloat(imageHeight) - inkTop) / CGFloat(imageHeight)),
        round6((x1 - x0) / CGFloat(imageWidth)),
        round6((inkTop - inkBottom) / CGFloat(imageHeight)),
    ]
    expected.append(["kind": kind, "text": value, "box": box])
}

guard let upright = ctx.makeImage() else {
    fail("could not rasterise the fixture")
}

func writePNG(_ image: CGImage, to url: URL) throws {
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
        throw NSError(domain: "fixture", code: 1, userInfo: [NSLocalizedDescriptionKey: "PNG destination failed"])
    }
    CGImageDestinationAddImage(dest, image, [kCGImagePropertyOrientation: 1] as CFDictionary)
    guard CGImageDestinationFinalize(dest) else {
        throw NSError(domain: "fixture", code: 2, userInfo: [NSLocalizedDescriptionKey: "PNG finalize failed"])
    }
}

/// Stored pixels are the upright picture rotated 90° CCW, so a viewer honouring
/// EXIF orientation 6 (rotate 90° CW for display) shows the upright picture.
func rotatedForEXIF6(_ image: CGImage, space: CGColorSpace) -> CGImage? {
    let uprightW = CGFloat(image.width)
    let uprightH = CGFloat(image.height)
    guard let rotatedCtx = CGContext(
        data: nil,
        width: image.height,
        height: image.width,
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: space,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        return nil
    }
    rotatedCtx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    rotatedCtx.fill(CGRect(x: 0, y: 0, width: image.height, height: image.width))
    // (u, v) upright -> (uprightH - v, u) stored
    rotatedCtx.concatenate(CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: uprightH, ty: 0))
    rotatedCtx.draw(image, in: CGRect(x: 0, y: 0, width: uprightW, height: uprightH))
    return rotatedCtx.makeImage()
}

func writeJPEGOrientation6(_ image: CGImage, to url: URL, quality: Double) throws {
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else {
        throw NSError(domain: "fixture", code: 3, userInfo: [NSLocalizedDescriptionKey: "JPEG destination failed"])
    }
    let props: [CFString: Any] = [
        kCGImagePropertyOrientation: 6,
        kCGImageDestinationLossyCompressionQuality: quality,
    ]
    CGImageDestinationAddImage(dest, image, props as CFDictionary)
    guard CGImageDestinationFinalize(dest) else {
        throw NSError(domain: "fixture", code: 4, userInfo: [NSLocalizedDescriptionKey: "JPEG finalize failed"])
    }
}

do {
    try writePNG(upright, to: outDir.appendingPathComponent("redact-sample.png"))

    guard let rotated = rotatedForEXIF6(upright, space: colorSpace) else {
        fail("could not rotate the fixture for EXIF orientation 6")
    }
    try writeJPEGOrientation6(
        rotated,
        to: outDir.appendingPathComponent("redact-sample-rot6.jpg"),
        quality: 0.9
    )

    let entries: [String] = expected.map { entry in
        let kind = entry["kind"] as? String ?? ""
        let value = entry["text"] as? String ?? ""
        let box = entry["box"] as? [Double] ?? []
        let numbers = box.map { String(format: "%.6f", $0) }.joined(separator: ", ")
        return "  {\n    \"kind\": \"\(kind)\",\n    \"text\": \"\(value)\",\n    \"box\": [\(numbers)]\n  }"
    }
    let body = entries.enumerated()
        .map { index, entry in index + 1 == entries.count ? entry : entry + "," }
        .joined(separator: "\n")
    let json = "[\n" + body + "\n]\n"
    try Data(json.utf8).write(to: outDir.appendingPathComponent("redact-sample.expected.json"), options: .atomic)
} catch {
    fail(error.localizedDescription)
}

print("Wrote redact-sample.png, redact-sample-rot6.jpg and redact-sample.expected.json to \(outDir.path)")
