#!/usr/bin/env swift
// Generates the PDF fixtures of the PDF/A engine tests (all content is fake).
// Usage: swift scripts/make-pdfa-fixture.swift <out-dir>
//
// Writes into <out-dir>:
//   pdfa-source-links.pdf  two A5 pages of selectable text (well over 200 characters) and two Link
//                          annotations, one per page, both WITHOUT the Print flag: that is the case
//                          in which Ghostscript's PDF/A mode silently drops every link.
//   pdfa-locked.pdf        the same document protected by a password (a made-up test password).
//
// pdfa-sample-2b.pdf is not made here: it is the real Ghostscript result of pdfa-source-links.pdf,
// produced once by hand (see src-tauri/tests/pdfa_bridge.rs, the ignored test `real_ghostscript_run`).
//
// The pages are written by hand with the standard Helvetica font (a Type 1 font that is not embedded) and
// NOT drawn with CoreText: CoreGraphics embeds system fonts such as HelveticaNeue as TrueType subsets without
// a Unicode map, and Ghostscript's PDF/A mode then turns them into fonts from which PDFKit can read almost no
// text (measured: 1 of 537 characters). That is the case the engine's text guard refuses; this fixture is the
// ordinary case, which keeps its text (measured: 100 %).

import Foundation
import CoreGraphics
import PDFKit

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard CommandLine.arguments.count >= 2 else {
    fail("usage: swift scripts/make-pdfa-fixture.swift <out-dir>")
}
let outDir = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
try FileManager.default.createDirectory(at: outDir, withIntermediateDirectories: true)

let pageWidth = 420
let pageHeight = 595
let margin = 40
let documentTitle = "FLIP PDF/A fixture (fake data)"
let fixtureUserPassword = "flip-fixture-user"
let fixtureOwnerPassword = "flip-fixture-owner"

struct PageLine {
    let text: String
    let isHeading: Bool
}

let pages: [[PageLine]] = [
    [
        PageLine(text: "FLIP FIXTURE MANUAL, PAGE ONE", isHeading: true),
        PageLine(text: "This is a made-up manual used only by the PDF/A tests.", isHeading: false),
        PageLine(text: "The quick brown fox jumps over the lazy dog, twice.", isHeading: false),
        PageLine(text: "Chapter one explains how to install the imaginary widget.", isHeading: false),
        PageLine(text: "Chapter two explains how to remove the imaginary widget.", isHeading: false),
        PageLine(text: "Caffè, perché, città e naïve stay readable as text.", isHeading: false),
        PageLine(text: "Visit the example site (link below) or go to page two.", isHeading: false),
    ],
    [
        PageLine(text: "PAGE TWO: THE APPENDIX", isHeading: true),
        PageLine(text: "Nothing here is real: names, numbers and places are invented.", isHeading: false),
        PageLine(text: "Part number 0001-FAKE, serial 123456, revision A.", isHeading: false),
        PageLine(text: "Selectable text must still be selectable after conversion.", isHeading: false),
        PageLine(text: "Searching this page for the word appendix must find it.", isHeading: false),
        PageLine(text: "Go back to page one with the link below.", isHeading: false),
    ],
]

/// A PDF string literal in WinAnsi, with the three characters that need a backslash escaped.
func pdfString(_ text: String) -> Data {
    guard let bytes = text.data(using: .windowsCP1252) else { fail("not representable in WinAnsi: \(text)") }
    var out = Data([UInt8(ascii: "(")])
    for byte in bytes {
        if byte == UInt8(ascii: "(") || byte == UInt8(ascii: ")") || byte == UInt8(ascii: "\\") {
            out.append(UInt8(ascii: "\\"))
        }
        out.append(byte)
    }
    out.append(UInt8(ascii: ")"))
    return out
}

/// The content stream of one page: a bold heading and body lines in standard Helvetica.
func contentStream(for lines: [PageLine]) -> Data {
    var stream = Data("BT\n".utf8)
    var baseline = pageHeight - margin
    for line in lines {
        baseline -= line.isHeading ? 30 : 24
        stream.append(Data("/\(line.isHeading ? "F2 20" : "F1 13") Tf 1 0 0 1 \(margin) \(baseline) Tm ".utf8))
        stream.append(pdfString(line.text))
        stream.append(Data(" Tj\n".utf8))
    }
    stream.append(Data("ET\n".utf8))
    return stream
}

/// The Link annotation of each page; neither carries the Print flag, in the two ways a file can lack it.
/// Page one links to a web address and has no `/F` entry at all (PDFKit still reports `shouldPrint == true` for
/// it, Ghostscript drops it). Page two links back to page one (object 5 is the first page) with an explicit `/F 0`.
let linkAnnotations: [String] = [
    "<< /Type /Annot /Subtype /Link /Rect [\(margin) 120 260 138] /Border [0 0 0] "
        + "/A << /S /URI /URI (https://example.com/flip-fixture) >> >>",
    "<< /Type /Annot /Subtype /Link /Rect [\(margin) 120 260 138] /Border [0 0 0] /F 0 "
        + "/Dest [5 0 R /XYZ 0 \(pageHeight) null] >>",
]

/// A complete, valid PDF: catalog, page tree, two standard fonts, and per page a page object, its content
/// stream and its link annotation (objects 5 + 3n, 6 + 3n and 7 + 3n).
/// The links are written here, not added with PDFKit: PDFKit serialises an internal destination as a name
/// (`/Dest [ /#231 /XYZ … ]`) instead of a page reference, which is a broken link that Ghostscript rightly drops.
func rawPDF(pages: [[PageLine]]) -> Data {
    var objects: [Data] = []
    let kids = (0..<pages.count).map { "\(5 + 3 * $0) 0 R" }.joined(separator: " ")
    objects.append(Data("<< /Type /Catalog /Pages 2 0 R >>".utf8))
    objects.append(Data("<< /Type /Pages /Kids [\(kids)] /Count \(pages.count) >>".utf8))
    objects.append(Data("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".utf8))
    objects.append(Data("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".utf8))
    for (index, lines) in pages.enumerated() {
        objects.append(Data((
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 \(pageWidth) \(pageHeight)] "
            + "/Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> "
            + "/Contents \(6 + 3 * index) 0 R /Annots [\(7 + 3 * index) 0 R] >>"
        ).utf8))
        let stream = contentStream(for: lines)
        var content = Data("<< /Length \(stream.count) >>\nstream\n".utf8)
        content.append(stream)
        content.append(Data("endstream".utf8))
        objects.append(content)
        objects.append(Data(linkAnnotations[index].utf8))
    }
    // The document information dictionary, with the title that the conversion must keep (the last object).
    var info = Data("<< /Title ".utf8)
    info.append(pdfString(documentTitle))
    info.append(Data(" >>".utf8))
    objects.append(info)

    var file = Data("%PDF-1.4\n".utf8)
    var offsets: [Int] = []
    for (index, object) in objects.enumerated() {
        offsets.append(file.count)
        file.append(Data("\(index + 1) 0 obj\n".utf8))
        file.append(object)
        file.append(Data("\nendobj\n".utf8))
    }
    let xrefOffset = file.count
    file.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
    for offset in offsets {
        file.append(Data(String(format: "%010d 00000 n \n", offset).utf8))
    }
    file.append(Data((
        "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R /Info \(objects.count) 0 R >>\n"
        + "startxref\n\(xrefOffset)\n%%EOF\n"
    ).utf8))
    return file
}

// 1. Write the pages and their two links into a draft file.
let draftURL = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
    .appendingPathComponent("flip-pdfa-fixture-draft-\(ProcessInfo.processInfo.processIdentifier).pdf")
try? FileManager.default.removeItem(at: draftURL)
defer { try? FileManager.default.removeItem(at: draftURL) }
try rawPDF(pages: pages).write(to: draftURL)

// 2. Open it with PDFKit; PDFKit writes both fixtures (the second one locked).
guard let doc = PDFDocument(url: draftURL), doc.pageCount == 2 else {
    fail("could not reopen the draft PDF")
}

let sourceURL = outDir.appendingPathComponent("pdfa-source-links.pdf")
let lockedURL = outDir.appendingPathComponent("pdfa-locked.pdf")
try? FileManager.default.removeItem(at: sourceURL)
try? FileManager.default.removeItem(at: lockedURL)

guard doc.write(to: sourceURL) else { fail("could not write \(sourceURL.path)") }
guard doc.write(to: lockedURL, withOptions: [
    PDFDocumentWriteOption.userPasswordOption: fixtureUserPassword,
    PDFDocumentWriteOption.ownerPasswordOption: fixtureOwnerPassword,
]) else { fail("could not write \(lockedURL.path)") }

// 3. Check what was written, so a PDFKit change can never silently break the tests.
guard let source = PDFDocument(url: sourceURL), source.pageCount == 2, !source.isLocked else {
    fail("the written source fixture cannot be reopened with 2 pages")
}
/// How many annotations of `url` have no Print flag in the file itself (`/F` absent or its bit 4 clear).
func annotationsWithoutPrintFlag(_ url: URL) -> Int {
    guard let cg = CGPDFDocument(url as CFURL) else { fail("cannot read \(url.path) with CoreGraphics") }
    var count = 0
    for pageNumber in 1...max(cg.numberOfPages, 1) {
        guard let pageDict = cg.page(at: pageNumber)?.dictionary else { continue }
        var annots: CGPDFArrayRef? = nil
        guard CGPDFDictionaryGetArray(pageDict, "Annots", &annots), let annots = annots else { continue }
        for index in 0..<CGPDFArrayGetCount(annots) {
            var annotation: CGPDFDictionaryRef? = nil
            guard CGPDFArrayGetDictionary(annots, index, &annotation), let annotation = annotation else { continue }
            var flags: CGPDFInteger = 0
            _ = CGPDFDictionaryGetInteger(annotation, "F", &flags)
            if flags & 4 == 0 { count += 1 }
        }
    }
    return count
}
guard (source.documentAttributes?[PDFDocumentAttribute.titleAttribute] as? String) == documentTitle else {
    fail("the source fixture lost its title")
}
guard annotationsWithoutPrintFlag(sourceURL) == 2 else {
    fail("both links of the source fixture must lack the Print flag in the file")
}

var annotationCount = 0
for index in 0..<source.pageCount {
    for annotation in source.page(at: index)?.annotations ?? [] {
        annotationCount += 1
        if index == 0 && annotation.url == nil { fail("the link on page 1 lost its web address") }
        if index == 1 {
            guard let target = annotation.destination?.page, source.index(for: target) == 0 else {
                fail("the link on page 2 does not lead to page 1")
            }
        }
    }
}
guard annotationCount == 2 else { fail("expected 2 annotations in the source fixture, found \(annotationCount)") }

var characters = 0
for index in 0..<source.pageCount {
    characters += (source.page(at: index)?.string ?? "").reduce(0) { $1.isWhitespace ? $0 : $0 + 1 }
}
guard characters > 400 else { fail("the source fixture has only \(characters) text characters") }

guard let locked = PDFDocument(url: lockedURL), locked.isLocked else {
    fail("the locked fixture is not locked")
}

print("Wrote \(sourceURL.path): 2 pages, \(characters) text characters, \(annotationCount) links without the Print flag")
print("Wrote \(lockedURL.path): password protected")
