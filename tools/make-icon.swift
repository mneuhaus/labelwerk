// Draws the app icon (graphite tile, centimetre grid, a signal-yellow tape with "Lw") and writes
// crates/labelwerk-app/assets/Labelwerk.icns. Run: swift tools/make-icon.swift
import AppKit
import CoreText

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let fontURL = root.appendingPathComponent("crates/labelwerk-core/fonts/BarlowSemiCondensed-Bold.ttf")
CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, nil)

func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(red: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: alpha)
}

func draw(size: Int) -> Data {
    let s = CGFloat(size) / 1024
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.scaleBy(x: s, y: s)

    // macOS icon grid: 824 pt tile with a soft drop shadow
    let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
    let shape = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: rgb(0x000000, 0.35))
    ctx.addPath(shape)
    ctx.setFillColor(rgb(0x1d1f23))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(shape)
    ctx.clip()
    let gradient = CGGradient(colorsSpace: nil, colors: [rgb(0x2e3136), rgb(0x1a1c1f)] as CFArray,
                              locations: [0, 1])!
    ctx.drawLinearGradient(gradient, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])

    // centimetre grid
    for i in stride(from: 100, through: 924, by: 103) {
        let major = (i - 100) % 412 == 0
        ctx.setStrokeColor(rgb(0xffffff, major ? 0.10 : 0.05))
        ctx.setLineWidth(major ? 3 : 2)
        ctx.move(to: CGPoint(x: CGFloat(i), y: 100)); ctx.addLine(to: CGPoint(x: CGFloat(i), y: 924))
        ctx.move(to: CGPoint(x: 100, y: CGFloat(i))); ctx.addLine(to: CGPoint(x: 924, y: CGFloat(i)))
        ctx.strokePath()
    }

    // the tape, edge to edge across the tile
    let tape = CGRect(x: 60, y: 362, width: 904, height: 300)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: rgb(0x000000, 0.45))
    ctx.setFillColor(rgb(0xf2c230))
    ctx.fill(tape)
    ctx.restoreGState()
    let sheen = CGGradient(colorsSpace: nil, colors: [rgb(0xffffff, 0.18), rgb(0xffffff, 0)] as CFArray,
                           locations: [0, 1])!
    ctx.saveGState()
    ctx.clip(to: tape)
    ctx.drawLinearGradient(sheen, start: CGPoint(x: 512, y: tape.maxY), end: CGPoint(x: 512, y: tape.midY), options: [])
    ctx.restoreGState()

    // cut lines
    ctx.setStrokeColor(rgb(0x18191b, 0.55))
    ctx.setLineWidth(5)
    ctx.setLineDash(phase: 0, lengths: [22, 16])
    for x: CGFloat in [214, 810] {
        ctx.move(to: CGPoint(x: x, y: tape.minY + 26)); ctx.addLine(to: CGPoint(x: x, y: tape.maxY - 26))
    }
    ctx.strokePath()
    ctx.setLineDash(phase: 0, lengths: [])

    // "Lw"
    let font = CTFontCreateWithName("BarlowSemiCondensed-Bold" as CFString, 270, nil)
    let text = NSAttributedString(string: "Lw", attributes: [
        NSAttributedString.Key(kCTFontAttributeName as String): font,
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): rgb(0x18191b),
    ])
    let line = CTLineCreateWithAttributedString(text)
    let bounds = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
    ctx.textPosition = CGPoint(x: 512 - bounds.midX, y: tape.midY - bounds.midY)
    CTLineDraw(line, ctx)
    ctx.restoreGState()

    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("Labelwerk.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    try! draw(size: base).write(to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    try! draw(size: base * 2).write(to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}
let out = root.appendingPathComponent("crates/labelwerk-app/assets/Labelwerk.icns")
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", out.path]
try! iconutil.run()
iconutil.waitUntilExit()
try! draw(size: 512).write(to: root.appendingPathComponent("docs/icon.png"))
print("wrote \(out.path) and docs/icon.png")
