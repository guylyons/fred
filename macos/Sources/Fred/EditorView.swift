import AppKit
import FredKit

/// Draws core frames with Core Text: opaque layer, pixel-snapped glyphs,
/// only changed rows redrawn.
final class EditorView: NSView {
    var font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular) { didSet { metricsChanged() } }
    var palette = Palette(fg: NSColor(srgbRed: 0xab / 255, green: 0xb2 / 255, blue: 0xbf / 255, alpha: 1),
                          bg: NSColor(srgbRed: 0x28 / 255, green: 0x2c / 255, blue: 0x34 / 255, alpha: 1)) {
        didSet { lines.removeAll(); needsDisplay = true }
    }
    /// Called with (cols, rows) whenever the size in cells changes.
    var onResize: (Int, Int) -> Void = { _, _ in }
    /// Called after each draw with the frame on screen (latency hook).
    var onDrawn: (Frame) -> Void = { _ in }

    /// Sends a message to the core (wired by main.swift).
    var send: (Outgoing) -> Void = { _ in }
    /// Every seq sent, with its send time (CACurrentMediaTime), for latency measurement.
    private(set) var sent: [(seq: Int, at: CFTimeInterval)] = []
    private var lastSeq = 0
    private var scrollAcc: CGFloat = 0

    /// Sends the message `make` builds from the next seq; returns that seq.
    @discardableResult
    func transmit(at: CFTimeInterval = CACurrentMediaTime(), _ make: (Int) -> Outgoing) -> Int {
        lastSeq += 1
        sent.append((lastSeq, at))
        if sent.count > 4096 { sent.removeFirst(2048) } // ponytail: bounded log, Task 8 drains it
        send(make(lastSeq))
        return lastSeq
    }

    private(set) var current: Frame?
    private(set) var cell = CGSize(width: 8, height: 16)
    private var scale: CGFloat { window?.backingScaleFactor ?? 2 }
    private var lastSize = (0, 0)
    private var fonts: [NSFont] = [] // index: bold | italic << 1
    private var lines: [LineKey: CTLine] = [:]

    private struct LineKey: Hashable {
        var s: String, fg: Int?, bg: Int?, f: Int
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.isOpaque = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        metricsChanged()
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isOpaque: Bool { true }
    override var isFlipped: Bool { true }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        layer?.contentsScale = scale
        metricsChanged()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        layer?.contentsScale = scale
        metricsChanged()
    }

    override func setFrameSize(_ s: NSSize) {
        super.setFrameSize(s)
        reportSize()
    }

    private func metricsChanged() {
        cell = cellSize(font: font, scale: scale)
        let d = font.fontDescriptor
        let bold = NSFont(descriptor: d.withSymbolicTraits(.bold), size: font.pointSize) ?? font
        func italic(_ f: NSFont) -> NSFont {
            let it = NSFont(descriptor: f.fontDescriptor.withSymbolicTraits(.italic), size: f.pointSize)
            if let it, it.fontDescriptor.symbolicTraits.contains(.italic) { return it }
            var oblique = CGAffineTransform(a: 1, b: 0, c: 0.21, d: 1, tx: 0, ty: 0) // ~12°
            return CTFontCreateWithFontDescriptor(f.fontDescriptor, f.pointSize, &oblique)
        }
        fonts = [font, bold, italic(font), italic(bold)]
        lines.removeAll()
        needsDisplay = true
        reportSize()
    }

    private func reportSize() {
        let size = (Int(bounds.width / cell.width), Int(bounds.height / cell.height))
        guard window != nil, size != lastSize, size.0 > 0, size.1 > 0 else { return }
        lastSize = size
        onResize(size.0, size.1)
    }

    // MARK: - Input

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    // ponytail: no NSTextInputClient yet, so no IME or dead keys; committed text is the event's chars.
    override func keyDown(with e: NSEvent) {
        let t = CACurrentMediaTime()
        let m = e.modifierFlags
        guard let k = notation(chars: e.characters ?? "", charsIgnoringModifiers: e.charactersIgnoringModifiers ?? "",
                               keyCode: e.keyCode, control: m.contains(.control), option: m.contains(.option),
                               shift: m.contains(.shift), command: m.contains(.command)) else { return }
        transmit(at: t) { .keys(seq: $0, keys: k) }
    }

    private func cellAt(_ e: NSEvent) -> (Int, Int) {
        let p = convert(e.locationInWindow, from: nil)
        let cols = current?.cols ?? Int.max, rows = current?.rows ?? Int.max
        return (min(max(0, Int(p.x / cell.width)), cols - 1), min(max(0, Int(p.y / cell.height)), rows - 1))
    }

    private func mouse(_ kind: String, _ e: NSEvent) {
        let (c, r) = cellAt(e)
        transmit { .mouse(seq: $0, kind: kind, col: c, row: r) }
    }

    override func mouseDown(with e: NSEvent) { mouse(e.modifierFlags.contains(.control) ? "right" : "down", e) }
    override func mouseDragged(with e: NSEvent) { mouse("drag", e) }
    override func mouseUp(with e: NSEvent) { mouse("up", e) }
    override func rightMouseDown(with e: NSEvent) { mouse("right", e) }

    override func scrollWheel(with e: NSEvent) {
        scrollAcc += e.scrollingDeltaY
        while abs(scrollAcc) >= 3 {
            mouse(scrollAcc > 0 ? "scroll_up" : "scroll_down", e)
            scrollAcc -= scrollAcc > 0 ? 3 : -3
        }
    }

    // MARK: - Frames

    func apply(_ f: Frame) {
        let old = current
        current = f
        guard let old, old.cols == f.cols, old.rows == f.rows, old.windows.count == f.windows.count,
              zip(old.windows, f.windows).allSatisfy({ sameRect($0, $1) })
        else { needsDisplay = true; return }
        for (o, n) in zip(old.windows, f.windows) {
            for r in 0..<n.rows.count where r >= o.rows.count || o.rows[r] != n.rows[r] {
                setNeedsDisplay(rowRect(n.y + r))
            }
        }
        let top = bottomTop(f)
        for r in 0..<max(old.bottom.count, f.bottom.count)
        where r >= old.bottom.count || r >= f.bottom.count || old.bottom[r] != f.bottom[r] {
            setNeedsDisplay(rowRect(top + r))
        }
        if old.cursor != f.cursor {
            if let c = old.cursor { setNeedsDisplay(rowRect(c.y)) }
            if let c = f.cursor { setNeedsDisplay(rowRect(c.y)) }
        }
    }

    private func sameRect(_ a: Win, _ b: Win) -> Bool {
        a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h && a.divider == b.divider
    }

    private func bottomTop(_ f: Frame) -> Int { f.windows.map { $0.y + $0.h }.max() ?? 0 }

    private func rowRect(_ y: Int) -> NSRect {
        NSRect(x: 0, y: CGFloat(y) * cell.height, width: bounds.width, height: cell.height)
    }

    // MARK: - Drawing

    override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(palette.bg.cgColor)
        ctx.fill(dirty)
        guard let f = current else { return }
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        for w in f.windows {
            for (r, runs) in w.rows.enumerated() where needsToDraw(rowRect(w.y + r)) {
                drawRow(ctx, runs, col: w.x, row: w.y + r)
            }
            // The last row is left for the StatusBar view (Task 8).
            if w.divider {
                let x = snap((CGFloat(w.x + w.w - 1) + 0.5) * cell.width, scale: scale)
                ctx.setFillColor(NSColor.separatorColor.cgColor)
                ctx.fill(CGRect(x: x, y: CGFloat(w.y) * cell.height, width: 1 / scale,
                                height: CGFloat(w.h) * cell.height))
            }
        }
        let top = bottomTop(f)
        for (r, runs) in f.bottom.enumerated() where needsToDraw(rowRect(top + r)) {
            drawRow(ctx, runs, col: 0, row: top + r)
        }
        if let c = f.cursor, needsToDraw(rowRect(c.y)) { drawCursor(ctx, c, f) }
        onDrawn(f)
    }

    /// Left and right edges of `n` cells from `col`, on device pixels.
    private func span(_ col: Int, _ n: Int) -> (CGFloat, CGFloat) {
        (snap(CGFloat(col) * cell.width, scale: scale), snap(CGFloat(col + n) * cell.width, scale: scale))
    }

    private func baseline(_ row: Int) -> CGFloat {
        let pad = (cell.height - (font.ascender - font.descender + font.leading)) / 2
        return snap(CGFloat(row) * cell.height + pad + font.ascender, scale: scale)
    }

    private func drawRow(_ ctx: CGContext, _ runs: [Run], col: Int, row: Int) {
        let y = CGFloat(row) * cell.height
        for run in runs {
            let flags = run.f ?? 0
            guard flags & 8 != 0 || run.bg != nil else { continue }
            let (l, r) = span(col + run.x, cellWidth(run.s))
            ctx.setFillColor(colors(run).bg.cgColor)
            ctx.fill(CGRect(x: l, y: y, width: r - l, height: cell.height))
        }
        let base = baseline(row)
        for run in runs {
            let flags = run.f ?? 0
            let (l, r) = span(col + run.x, cellWidth(run.s))
            func key(_ s: String) -> LineKey { LineKey(s: s, fg: run.fg, bg: flags & 8 != 0 ? run.bg : nil, f: flags) }
            if cellWidth(run.s) == run.s.count {
                ctx.textPosition = CGPoint(x: l, y: base)
                CTLineDraw(line(key(run.s)), ctx)
            } else {
                // Fallback fonts' wide glyphs aren't 2 cells wide: place each character on the grid.
                var at = col + run.x
                for ch in run.s {
                    let s = String(ch)
                    ctx.textPosition = CGPoint(x: span(at, 1).0, y: base)
                    CTLineDraw(line(key(s)), ctx)
                    at += cellWidth(s)
                }
            }
            if flags & (4 | 32) != 0 {
                ctx.setFillColor(colors(run).fg.cgColor)
                if flags & 4 != 0 {
                    ctx.fill(CGRect(x: l, y: snap(base - font.underlinePosition, scale: scale), width: r - l, height: 1 / scale))
                }
                if flags & 32 != 0 {
                    ctx.fill(CGRect(x: l, y: snap(base - font.xHeight / 2, scale: scale), width: r - l, height: 1 / scale))
                }
            }
        }
    }

    private func colors(_ run: Run) -> (fg: NSColor, bg: NSColor) { colors(run.fg, run.bg, run.f ?? 0) }

    private func colors(_ fgCode: Int?, _ bgCode: Int?, _ flags: Int) -> (fg: NSColor, bg: NSColor) {
        var fg = palette.color(fgCode, default: palette.fg)
        var bg = palette.color(bgCode, default: palette.bg)
        if flags & 8 != 0 { swap(&fg, &bg) }
        if flags & 16 != 0 { fg = fg.withAlphaComponent(0.6) }
        return (fg, bg)
    }

    private func line(_ k: LineKey) -> CTLine {
        if let l = lines[k] { return l }
        if lines.count > 4096 { lines.removeAll() } // ponytail: wholesale eviction, LRU if it ever churns
        let attrs: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): fonts[k.f & 3],
            NSAttributedString.Key(kCTForegroundColorAttributeName as String): colors(k.fg, k.bg, k.f).fg.cgColor,
        ]
        let l = CTLineCreateWithAttributedString(NSAttributedString(string: k.s, attributes: attrs))
        lines[k] = l
        return l
    }

    private func drawCursor(_ ctx: CGContext, _ c: Cursor, _ f: Frame) {
        let under = character(at: c.x, c.y, f)
        let (l, r) = span(under?.col ?? c.x, under?.width ?? 1)
        let y = CGFloat(c.y) * cell.height
        switch c.shape {
        case "bar":
            ctx.setFillColor(palette.fg.cgColor)
            ctx.fill(CGRect(x: l, y: y, width: 2, height: cell.height))
        case "underline":
            ctx.setFillColor(palette.fg.cgColor)
            ctx.fill(CGRect(x: l, y: y + cell.height - 2, width: r - l, height: 2))
        default:
            ctx.setFillColor(palette.fg.withAlphaComponent(0.85).cgColor)
            ctx.fill(CGRect(x: l, y: y, width: r - l, height: cell.height))
            if let (ch, flags, _, _) = under, ch != " " {
                // Reversed with default colors draws in the theme bg.
                ctx.textPosition = CGPoint(x: l, y: baseline(c.y))
                CTLineDraw(line(LineKey(s: ch, fg: nil, bg: nil, f: (flags & 3) | 8)), ctx)
            }
        }
    }

    /// The character at absolute cell (x, y): its run's flags, absolute start column and width.
    private func character(at x: Int, _ y: Int, _ f: Frame) -> (s: String, flags: Int, col: Int, width: Int)? {
        var origin = 0
        var runs: [Run]?, col = x
        for w in f.windows where x >= w.x && x < w.x + w.w && y >= w.y && y - w.y < w.rows.count {
            runs = w.rows[y - w.y]; col = x - w.x; origin = w.x
        }
        let top = bottomTop(f)
        if runs == nil, y >= top, y - top < f.bottom.count { runs = f.bottom[y - top] }
        for run in runs ?? [] where col >= run.x {
            var at = run.x
            for ch in run.s {
                let n = cellWidth(String(ch))
                if col < at + n { return (String(ch), run.f ?? 0, origin + at, n) }
                at += n
            }
        }
        return nil
    }
}
