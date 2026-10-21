import AppKit
import FredKit
import QuartzCore

/// Sublime's minimap over a window's margin columns: the buffer at 1 pt per
/// column and 2 pt per line, rendered once per `minimap` message, with the
/// visible lines under a faint box. A click centres the editor on the
/// clicked line; dragging the box scrolls.
final class MinimapView: NSView {
    /// Sends `scroll` with a new top for this view's window.
    var onScroll: (Int) -> Void = { _ in }
    var palette = Palette.materialDark

    private var map: MapImage?
    private var top = 0, visible = 0
    private var lines: Int { map?.lines ?? 0 }
    /// The pointer's distance below the box's top while dragging.
    private var grab: CGFloat?
    private var pending: Int?, sent: Int?
    private var link: CADisplayLink?

    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true // else the dirty rect can reach past us, over the text
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(_ map: MapImage?, top: Int, visible: Int) {
        guard map?.gen != self.map?.gen || top != self.top || visible != self.visible else { return }
        self.map = map
        self.top = top
        self.visible = visible
        needsDisplay = true
    }

    override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        needsDisplay = true
    }

    private var offset: CGFloat { MinimapMath.offset(top: top, lines: lines, visible: visible, height: bounds.height) }
    private var boxY: CGFloat { MinimapMath.boxY(top: top, lines: lines, visible: visible, height: bounds.height) }

    /// The whole map as one bitmap, runs batched by color.
    nonisolated static func render(_ runs: [MiniRun], lines: Int, palette: Palette, scale: CGFloat) -> CGImage? {
        guard lines > 0 else { return nil }
        let w = MinimapMath.width, h = CGFloat(lines) * MinimapMath.lineHeight
        // ponytail: one bitmap capped at ~8M pixels (lower resolution past ~20k lines); tile it if that blurs.
        let s = min(scale, (8_000_000 / (w * h)).squareRoot())
        guard let ctx = CGContext(data: nil, width: Int((w * s).rounded(.up)), height: Int((h * s).rounded(.up)),
                                  bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.translateBy(x: 0, y: CGFloat(ctx.height))
        ctx.scaleBy(x: s, y: -s)
        var byColor: [Int32?: [CGRect]] = [:]
        for r in runs {
            byColor[r.color, default: []].append(CGRect(x: CGFloat(r.col), y: CGFloat(r.line) * MinimapMath.lineHeight,
                                                         width: CGFloat(r.len), height: 1.5))
        }
        for (code, rects) in byColor {
            ctx.setFillColor(palette.color(code.map(Int.init), default: palette.fg)
                .withAlphaComponent(code == nil ? 0.45 : 0.75).cgColor)
            ctx.fill(rects)
        }
        return ctx.makeImage()
    }

    override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(palette.bg.cgColor)
        ctx.fill(dirty)
        guard let image = map?.image, lines > 0 else { return }
        // Only the visible slice: the whole map can be ~100k pt tall.
        let total = CGFloat(lines) * MinimapMath.lineHeight, off = offset
        let sliceH = min(bounds.height, total - off)
        let px = CGFloat(image.height) / total
        let y0 = (off * px).rounded(.down) // whole image pixels; the rest shifts the slice up
        if sliceH > 0, let slice = image.cropping(to: CGRect(x: 0, y: y0, width: CGFloat(image.width),
                                                             height: (sliceH * px).rounded(.up) + 1)) {
            let h = CGFloat(slice.height) / px
            ctx.saveGState()
            ctx.translateBy(x: 0, y: y0 / px - off + h) // the flipped view would draw the image upside down
            ctx.scaleBy(x: 1, y: -1)
            ctx.draw(slice, in: CGRect(x: 0, y: 0, width: CGFloat(slice.width) / px, height: h))
            ctx.restoreGState()
        }
        ctx.setFillColor(NSColor.white.withAlphaComponent(0.07).cgColor)
        ctx.fill(CGRect(x: 0, y: boxY, width: bounds.width, height: CGFloat(visible) * MinimapMath.lineHeight))
    }

    // MARK: - Mouse

    override func mouseDown(with e: NSEvent) {
        guard lines > 0 else { return }
        let y = convert(e.locationInWindow, from: nil).y, box = boxY, boxH = CGFloat(visible) * MinimapMath.lineHeight
        if y >= box && y < box + boxH {
            grab = y - box
        } else {
            let line = MinimapMath.line(atY: y, offset: offset)
            send(MinimapMath.topForCentering(line: line, visible: visible, lines: lines))
            grab = boxH / 2
        }
        link = displayLink(target: self, selector: #selector(tick))
        link?.add(to: .main, forMode: .common)
    }

    override func mouseDragged(with e: NSEvent) {
        guard let grab else { return }
        let y = convert(e.locationInWindow, from: nil).y - grab
        pending = MinimapMath.top(forBoxY: y, lines: lines, visible: visible, height: bounds.height)
    }

    override func mouseUp(with e: NSEvent) {
        tick()
        link?.invalidate()
        link = nil
        grab = nil
        sent = nil
    }

    /// At most one `scroll` per display frame while dragging.
    @objc private func tick() {
        guard let p = pending else { return }
        pending = nil
        send(p)
    }

    private func send(_ t: Int) {
        guard t != sent else { return }
        sent = t
        onScroll(t)
    }
}

/// One window's rendered map. The compact runs stay for a re-render at a
/// new backing scale; the decoded message doesn't.
struct MapImage: Sendable {
    var gen: Int
    var lines: Int
    var runs: [MiniRun]
    var image: CGImage?
    var scale: CGFloat
}
