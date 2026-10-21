import AppKit
import FredKit

/// Draws core frames with Core Text: opaque layer, pixel-snapped glyphs,
/// only changed rows redrawn.
final class EditorView: NSView {
    var font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular) { didSet { metricsChanged() } }
    var palette = Palette.materialDark {
        didSet { lines.removeAll(); needsDisplay = true }
    }
    /// Called with (cols, rows, minimap margin) whenever one of them changes.
    var onResize: (Int, Int, Int) -> Void = { _, _, _ in }
    /// View ▸ Show/Hide Minimap (set by main.swift).
    var showMinimap = true {
        didSet { reportSize(); if let f = current { layoutMinimaps(f) } }
    }
    private(set) var latency = Latency()
    private let showLatency = ProcessInfo.processInfo.environment["FRED_LATENCY"] == "1"
    private let overlay = NSTextField(labelWithString: "")
    private var printed = 0
    private var statusBars: [StatusBar] = []
    private var tabStrips: [TabStrip] = []
    private var minimapViews: [MinimapView] = []
    /// The rendered map per window id; `mapGen` and `mapPending` drop stale renders.
    private var maps: [Int: MapImage] = [:]
    private var mapGen = 0
    private var mapPending: [Int: Int] = [:]
    /// Reveal in Sidebar, from a tab's menu (wired by main.swift).
    var reveal: (String) -> Void = { _ in }

    /// Sends a message to the core (wired by main.swift).
    var send: (Outgoing) -> Void = { _ in }
    private var lastSeq = 0
    private var scrollAcc: CGFloat = 0

    /// Sends the message `make` builds from the next seq; returns that seq.
    @discardableResult
    /// Pass `at` (the keypress time) only for keypresses: those alone are latency samples.
    func transmit(at: CFTimeInterval? = nil, _ make: (Int) -> Outgoing) -> Int {
        lastSeq += 1
        if let at { latency.start(seq: lastSeq, at: at) }
        send(make(lastSeq))
        return lastSeq
    }

    private(set) var current: Frame?
    private(set) var cell = CGSize(width: 8, height: 16)
    private var scale: CGFloat { window?.backingScaleFactor ?? 2 }
    private var lastSize = (0, 0, 0)
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
        overlay.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        overlay.textColor = .white
        overlay.drawsBackground = true
        overlay.backgroundColor = NSColor.black.withAlphaComponent(0.6)
        overlay.isHidden = true
        overlay.autoresizingMask = [.minXMargin, .maxYMargin]
        addSubview(overlay)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isOpaque: Bool { true }
    override var isFlipped: Bool { true }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        layer?.contentsScale = scale
        metricsChanged()
        // Sharp again on the new display.
        for (id, m) in maps where m.scale != scale && mapPending[id] == nil {
            let runs = m.runs, lines = m.lines
            renderMap(id) { (lines, runs) }
        }
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
        let margin = showMinimap ? Int((MinimapMath.width / cell.width).rounded(.up)) : 0
        let size = (Int(bounds.width / cell.width), Int(bounds.height / cell.height), margin)
        guard window != nil, size != lastSize, size.0 > 0, size.1 > 0 else { return }
        lastSize = size
        onResize(size.0, size.1, size.2)
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
        let n = scrollLines(e.scrollingDeltaY, precise: e.hasPreciseScrollingDeltas, acc: &scrollAcc)
        for _ in 0..<abs(n) { mouse(n > 0 ? "scroll_up" : "scroll_down", e) }
    }

    // MARK: - Frames

    func apply(_ f: Frame) {
        let old = current
        current = f
        layoutStatusBars(f)
        layoutTabStrips(f)
        layoutMinimaps(f)
        defer { if !needsDisplay { frameShown(f) } } // nothing to draw: the echo is already on screen
        guard let old, old.cols == f.cols, old.rows == f.rows, old.windows.count == f.windows.count,
              zip(old.windows, f.windows).allSatisfy({ sameRect($0, $1) })
        else { needsDisplay = true; return }
        for (o, n) in zip(old.windows, f.windows) {
            for r in 0..<n.rows.count where r >= o.rows.count || o.rows[r] != n.rows[r] {
                setNeedsDisplay(rowRect(textY(n) + r))
            }
            // A changed selection: its rows and their neighbours (the corners
            // and the border reach them).
            if o.sel != n.sel {
                for s in (o.sel ?? []) + (n.sel ?? []) {
                    for r in s.y - 1...s.y + 1 { setNeedsDisplay(rowRect(textY(n) + r)) }
                }
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

    /// One StatusBar per window, over its last cell row (minus the divider column).
    private func layoutStatusBars(_ f: Frame) {
        while statusBars.count < f.windows.count {
            let b = StatusBar(frame: .zero)
            addSubview(b, positioned: .below, relativeTo: overlay)
            statusBars.append(b)
        }
        for (i, b) in statusBars.enumerated() {
            guard i < f.windows.count else { b.isHidden = true; continue }
            let w = f.windows[i]
            let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
            b.frame = NSRect(x: l, y: CGFloat(w.y + w.h - 1) * cell.height, width: r - l, height: cell.height)
            b.isHidden = false
            b.update(w.status, active: w.active)
        }
    }

    /// One TabStrip per window with a header row and tabs, over that row.
    private func layoutTabStrips(_ f: Frame) {
        while tabStrips.count < f.windows.count {
            let s = TabStrip(frame: .zero)
            s.editorBg = palette.bg
            s.reveal = { [weak self] in self?.reveal($0) }
            addSubview(s, positioned: .below, relativeTo: overlay)
            tabStrips.append(s)
        }
        for (i, s) in tabStrips.enumerated() {
            guard i < f.windows.count, case let w = f.windows[i], let id = w.id, let tabs = w.tabs, (w.header ?? 0) > 0 else {
                s.isHidden = true
                continue
            }
            let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
            let f = NSRect(x: l, y: CGFloat(w.y) * cell.height, width: r - l, height: CGFloat(w.header!) * cell.height)
            if s.frame != f { s.frame = f }
            s.isHidden = false
            s.op = { [weak self] op, index, to in self?.transmit { .tab(seq: $0, win: id, op: op, index: index, to: to) } }
            s.update(tabs, active: w.active)
        }
    }

    /// Every message renders: the core sends one only on a change, and
    /// buffer versions aren't unique across buffers.
    func applyMinimap(_ m: Minimap) {
        guard let rows = m.lines else {
            maps[m.win] = nil // too big: an empty margin
            mapPending[m.win] = nil
            if let f = current { layoutMinimaps(f) }
            return
        }
        renderMap(m.win) { (rows.count, MinimapMath.compact(rows)) }
    }

    /// Renders off the main thread (a long file takes tens of ms); the old
    /// map stays up until then, and a newer request wins.
    private func renderMap(_ id: Int, _ runs: @escaping @Sendable () -> (Int, [MiniRun])) {
        mapGen += 1
        let gen = mapGen, p = palette, s = scale
        mapPending[id] = gen
        let done: @MainActor (MapImage) -> Void = { [weak self] m in
            guard let self, self.mapPending[id] == gen else { return }
            self.mapPending[id] = nil
            self.maps[id] = m
            if let f = self.current { self.layoutMinimaps(f) }
        }
        DispatchQueue.global(qos: .userInitiated).async {
            let (lines, r) = runs()
            let m = MapImage(gen: gen, lines: lines, runs: r, image: MinimapView.render(r, lines: lines, palette: p, scale: s), scale: s)
            DispatchQueue.main.async { done(m) }
        }
    }

    /// One MinimapView per window with a margin, over its margin columns and text rows.
    private func layoutMinimaps(_ f: Frame) {
        let ids = Set(f.windows.compactMap(\.id))
        if maps.keys.contains(where: { !ids.contains($0) }) { maps = maps.filter { ids.contains($0.key) } }
        if mapPending.keys.contains(where: { !ids.contains($0) }) { mapPending = mapPending.filter { ids.contains($0.key) } }
        while minimapViews.count < f.windows.count {
            let m = MinimapView(frame: .zero)
            m.palette = palette
            addSubview(m, positioned: .below, relativeTo: overlay)
            minimapViews.append(m)
        }
        for (i, m) in minimapViews.enumerated() {
            guard showMinimap, i < f.windows.count, case let w = f.windows[i], let id = w.id, let margin = w.margin, margin > 0 else {
                m.isHidden = true
                continue
            }
            let (l, r) = span(w.x + w.w - (w.divider ? 1 : 0) - margin, margin)
            let visible = max(0, w.h - (w.header ?? 0) - 1) // the text rows
            let fr = NSRect(x: l, y: CGFloat(textY(w)) * cell.height, width: r - l, height: CGFloat(visible) * cell.height)
            if m.frame != fr { m.frame = fr }
            m.isHidden = false
            m.onScroll = { [weak self] top in self?.transmit { .scroll(seq: $0, win: id, top: top) } }
            m.update(maps[id], top: w.top ?? 0, visible: visible)
        }
    }

    /// The window's first text row: below its tab strip.
    private func textY(_ w: Win) -> Int { w.y + (w.header ?? 0) }

    private func frameShown(_ f: Frame) {
        guard latency.shown(seq: f.seq, at: CACurrentMediaTime()), showLatency else { return }
        if latency.total / 100 > printed {
            printed = latency.total / 100
            FileHandle.standardError.write(Data((latency.summary + "\n").utf8))
        }
        overlay.stringValue = latency.summary
        overlay.sizeToFit()
        overlay.frame.origin = NSPoint(x: bounds.maxX - overlay.frame.width - 8, y: 8)
        overlay.isHidden = false
    }

    private func sameRect(_ a: Win, _ b: Win) -> Bool {
        a.x == b.x && a.y == b.y && a.w == b.w && a.h == b.h && a.divider == b.divider && a.header == b.header
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
            let y = textY(w)
            let dirty = w.rows.indices.filter { needsToDraw(rowRect(y + $0)) }
            for r in dirty { drawBackgrounds(ctx, w.rows[r], col: w.x, row: y + r) }
            if let sel = w.sel, !sel.isEmpty { drawSelection(ctx, sel, w) }
            for r in dirty { drawText(ctx, w.rows[r], col: w.x, row: y + r) }
            if w.divider {
                let x = snap((CGFloat(w.x + w.w - 1) + 0.5) * cell.width, scale: scale)
                ctx.setFillColor(NSColor.separatorColor.cgColor)
                ctx.fill(CGRect(x: x, y: CGFloat(w.y) * cell.height, width: 1 / scale,
                                height: CGFloat(w.h) * cell.height))
            }
        }
        let top = bottomTop(f)
        for (r, runs) in f.bottom.enumerated() where needsToDraw(rowRect(top + r)) {
            drawBackgrounds(ctx, runs, col: 0, row: top + r)
            drawText(ctx, runs, col: 0, row: top + r)
        }
        if let c = f.cursor, needsToDraw(rowRect(c.y)) { drawCursor(ctx, c, f) }
        frameShown(f)
    }

    /// Left and right edges of `n` cells from `col`, on device pixels.
    private func span(_ col: Int, _ n: Int) -> (CGFloat, CGFloat) {
        (snap(CGFloat(col) * cell.width, scale: scale), snap(CGFloat(col + n) * cell.width, scale: scale))
    }

    private func baseline(_ row: Int) -> CGFloat {
        let pad = (cell.height - (font.ascender - font.descender + font.leading)) / 2
        return snap(CGFloat(row) * cell.height + pad + font.ascender, scale: scale)
    }

    private func drawBackgrounds(_ ctx: CGContext, _ runs: [Run], col: Int, row: Int) {
        let y = CGFloat(row) * cell.height
        for run in runs {
            let flags = run.f ?? 0
            guard flags & 8 != 0 || run.bg != nil else { continue }
            let (l, r) = span(col + run.x, cellWidth(run.s))
            ctx.setFillColor(colors(run).bg.cgColor)
            ctx.fill(CGRect(x: l, y: y, width: r - l, height: cell.height))
        }
    }

    /// Sublime's selection: rounded shapes behind the text, clipped to the
    /// window's text rows.
    private func drawSelection(_ ctx: CGContext, _ sel: [Sel], _ w: Win) {
        let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
        ctx.saveGState()
        let y = CGFloat(textY(w)) * cell.height
        ctx.clip(to: CGRect(x: l, y: y, width: r - l, height: CGFloat(w.rows.count) * cell.height))
        ctx.translateBy(x: l, y: y)
        let path = selectionPath(sel, cell: cell, scale: scale)
        ctx.addPath(path)
        ctx.setFillColor(palette.selFill.cgColor)
        ctx.fillPath()
        // A 1 pt border inside the shape: stroke 2 pt, clipped to the shape.
        ctx.addPath(path)
        ctx.clip()
        ctx.addPath(path)
        ctx.setStrokeColor(palette.selBorder.cgColor)
        ctx.setLineWidth(2)
        ctx.strokePath()
        ctx.restoreGState()
    }

    private func drawText(_ ctx: CGContext, _ runs: [Run], col: Int, row: Int) {
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
        for w in f.windows where x >= w.x && x < w.x + w.w && y >= textY(w) && y - textY(w) < w.rows.count {
            runs = w.rows[y - textY(w)]; col = x - w.x; origin = w.x
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
