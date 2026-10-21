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

    private var rendered: (win: Int, version: Int)?
    private var image: CGImage?
    private var lines = 0, top = 0, visible = 0
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

    func update(_ map: Minimap?, top: Int, visible: Int) {
        if map?.win != rendered?.win || map?.version != rendered?.version {
            rendered = map.map { ($0.win, $0.version) }
            if let map, let rows = map.lines {
                // The old image stays up until the new one is ready: no blank flash per edit.
                // Off the main thread: a long file takes tens of ms, which keys mustn't wait for.
                let p = palette, scale = window?.backingScaleFactor ?? 2
                let done: @MainActor (CGImage?) -> Void = { [weak self] img in
                    guard let self, self.rendered?.win == map.win, self.rendered?.version == map.version else { return }
                    self.image = img
                    self.lines = rows.count
                    self.needsDisplay = true
                }
                DispatchQueue.global(qos: .userInitiated).async {
                    let img = MinimapView.render(rows, palette: p, scale: scale)
                    DispatchQueue.main.async { done(img) }
                }
            } else {
                image = nil
                lines = 0
                needsDisplay = true
            }
        }
        guard top != self.top || visible != self.visible else { return }
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
    private nonisolated static func render(_ map: [[[Int?]]], palette: Palette, scale: CGFloat) -> CGImage? {
        guard !map.isEmpty else { return nil }
        let w = MinimapMath.width, h = CGFloat(map.count) * MinimapMath.lineHeight
        // ponytail: one bitmap capped at ~8M pixels (lower resolution past ~20k lines); tile it if that blurs.
        let s = min(scale, (8_000_000 / (w * h)).squareRoot())
        guard let ctx = CGContext(data: nil, width: Int((w * s).rounded(.up)), height: Int((h * s).rounded(.up)),
                                  bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.translateBy(x: 0, y: CGFloat(ctx.height))
        ctx.scaleBy(x: s, y: -s)
        var byColor: [Int?: [CGRect]] = [:]
        for (i, runs) in map.enumerated() {
            for r in runs where r.count >= 2 {
                guard let col = r[0], let len = r[1], col < Int(w), len > 0 else { continue }
                let y = CGFloat(i) * MinimapMath.lineHeight
                byColor[r.count > 2 ? r[2] : nil, default: []]
                    .append(CGRect(x: CGFloat(col), y: y, width: CGFloat(min(len, Int(w) - col)), height: 1.5))
            }
        }
        for (code, rects) in byColor {
            ctx.setFillColor(palette.color(code, default: palette.fg).withAlphaComponent(code == nil ? 0.45 : 0.75).cgColor)
            ctx.fill(rects)
        }
        return ctx.makeImage()
    }

    override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(palette.bg.cgColor)
        ctx.fill(dirty)
        guard let image, lines > 0 else { return }
        let h = CGFloat(lines) * MinimapMath.lineHeight
        ctx.saveGState()
        ctx.translateBy(x: 0, y: h - offset) // the flipped view would draw the image upside down
        ctx.scaleBy(x: 1, y: -1)
        ctx.draw(image, in: CGRect(x: 0, y: 0, width: MinimapMath.width, height: h))
        ctx.restoreGState()
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
