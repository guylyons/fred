import AppKit
import FredKit

/// Draws core frames with Core Text: opaque layer, pixel-snapped glyphs,
/// only changed rows redrawn.
final class EditorView: NSView {
    /// Source Code Pro when installed, else the system monospace.
    static func editorFont(_ size: CGFloat) -> NSFont {
        NSFont(name: "SourceCodePro-Regular", size: size) ?? .monospacedSystemFont(ofSize: size, weight: .regular)
    }
    static let defaultSize: CGFloat = 15
    var font = EditorView.editorFont(EditorView.defaultSize) { didSet { metricsChanged() } }
    var palette = Palette.materialDark {
        didSet {
            lines.removeAll()
            layer?.isOpaque = !palette.clear
            statusBars.forEach { $0.editorBg = palette.bg }
            tabStrips.forEach { $0.editorBg = palette.bg }
            crumbs.forEach { $0.editorBg = palette.bg }
            chatHeaders.forEach { $0.editorBg = palette.bg }
            sessionLists.forEach { $0.editorBg = palette.bg }
            minimapViews.forEach { $0.palette = palette }
            needsDisplay = true
        }
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
    private var crumbs: [Breadcrumbs] = []
    /// The chat window's VS Code Chat header and Sessions list, per window.
    private var chatHeaders: [ChatHeader] = []
    private var sessionLists: [SessionsList] = []
    private let picker = PaletteView(frame: .zero)
    /// The cursor's blink: solid since this time (the last frame), then a smooth fade.
    private var blinkFrom = CACurrentMediaTime()
    private var blinkAlpha: CGFloat = 1
    /// The project folder: breadcrumbs start below it.
    var root = ""
    private var minimapViews: [MinimapView] = []
    /// Where a dragged tab would land: a window, an edge half, or a strip gap.
    private let dropHint = NSView()
    private var hintShown = false
    /// The dragged tab's name, following the pointer.
    private let ghost = NSTextField(labelWithString: "")
    /// The rendered map per window id; `mapGen` and `mapPending` drop stale renders.
    private var maps: [Int: MapImage] = [:]
    private var mapGen = 0
    private var mapPending: [Int: Int] = [:]
    /// Reveal in Sidebar, from a tab's menu (wired by main.swift).
    var reveal: (String) -> Void = { _ in }

    /// Sends a message to the core (wired by main.swift).
    var send: (Outgoing) -> Void = { _ in }
    private var panelSlideFrom: CFTimeInterval?
    private var panelSlideTimer: Timer?
    private var paintTracking: NSTrackingArea?
    private var lastSeq = 0
    private var scrollAcc: CGFloat = 0
    /// Chrome's rubber band: a window's text pulled past its last line by
    /// `dy` points (up negative), springing back on release. The first line
    /// is a hard stop: scrolling up halts where the text begins.
    private var bounce: (win: Int, dy: CGFloat)?
    private var spring: CADisplayLink?
    private var springAt: CFTimeInterval = 0
    /// A spring has begun for this gesture: its momentum tail is swallowed,
    /// so it can't pull the text again once the spring lands.
    private var coasting = false
    /// VS Code's smooth trackpad scroll: window `win` is wanted `px` points
    /// down its lines. The core shows whole lines from `sent` (asked for in
    /// message `seq`); the text slides by the rest.
    private var smooth: (win: Int, px: CGFloat, sent: Int, seq: Int)?

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
        dropHint.wantsLayer = true
        dropHint.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.18).cgColor
        dropHint.layer?.borderColor = NSColor.controlAccentColor.withAlphaComponent(0.6).cgColor
        dropHint.layer?.borderWidth = 1
        dropHint.layer?.cornerRadius = 3
        dropHint.isHidden = true
        addSubview(dropHint)
        registerForDraggedTypes([.fileURL])
        ghost.font = .systemFont(ofSize: 13)
        ghost.drawsBackground = true
        ghost.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.9)
        ghost.wantsLayer = true
        ghost.layer?.cornerRadius = 4
        ghost.isHidden = true
        addSubview(ghost)
        let blink = Timer(timeInterval: 1.0 / 30, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.blinkTick() }
        }
        RunLoop.main.add(blink, forMode: .common)
    }

    /// Redraws just the cursor cell when its blink alpha changes; solid when the window isn't key.
    private func blinkTick() {
        let a = window?.isKeyWindow == true ? Blink.alpha(CACurrentMediaTime() - blinkFrom) : 1
        guard abs(a - blinkAlpha) > 0.02 || (a != blinkAlpha && (a == 1 || a == 0)), let c = current?.cursor else { return }
        blinkAlpha = a
        let (l, r) = span(max(0, c.x - 1), 3)
        let row = rowRect(c.y)
        setNeedsDisplay(NSRect(x: l, y: row.minY, width: r - l, height: row.height))
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isOpaque: Bool { !palette.clear }
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
        var p = convert(e.locationInWindow, from: nil)
        if let f = current, case let s = CGFloat(bottomTop(f) - 1) * cell.height, p.y >= s {
            p.y = p.y >= s + slack ? p.y - slack : s - 1 // the slack is the last text row's
        }
        // A slid window: the cell under the pointer is the one drawn there.
        if let w = current?.windows.first(where: { textRect($0).contains(p) }) { p.y -= offset(w) }
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
        if rubberBand(e) || slide(e) { return }
        // A terminal scrolls 3 lines a step: one per 3 lines of finger travel.
        let n = scrollLines(e.scrollingDeltaY, precise: e.hasPreciseScrollingDeltas, unit: 3 * cell.height, acc: &scrollAcc)
        for _ in 0..<abs(n) { mouse(n > 0 ? "scroll_up" : "scroll_down", e) }
    }

    /// A trackpad scroll past the end stretches that window instead; true when it took the event.
    private func rubberBand(_ e: NSEvent) -> Bool {
        let d = e.scrollingDeltaY
        let released = e.phase == .ended || e.phase == .cancelled
        if e.phase == .began || e.phase == .mayBegin { coasting = false }
        if coasting, !e.momentumPhase.isEmpty {
            if e.momentumPhase == .ended || e.momentumPhase == .cancelled { coasting = false }
            return true // momentum into an edge already sprang back
        }
        guard e.hasPreciseScrollingDeltas, let f = current else { return false }
        let (c, r) = cellAt(e)
        guard let w = f.windows.first(where: { c >= $0.x && c < $0.x + $0.w && r >= $0.y && r < $0.y + $0.h }),
              let id = w.id, let top = w.top, let lines = w.lines else { return false }
        let over = bounce?.win == id ? bounce!.dy : 0
        let px = smooth?.win == id ? smooth!.px : CGFloat(top) * cell.height
        let atEnd = px >= CGFloat(maxTop(w, top: top, lines: lines)) * cell.height && d < 0
        guard over != 0 || atEnd || (released && bounce != nil) else { return false }
        stopSpring()
        let dy = Rubber.pull(over, by: d)
        bounce = dy == 0 ? nil : (id, dy)
        redrawText(w)
        if released || !e.momentumPhase.isEmpty {
            // Momentum still to come (it follows a release) is the bounce's, not a new pull.
            coasting = e.momentumPhase != .ended && e.momentumPhase != .cancelled
            springBack()
        }
        return true
    }

    /// A trackpad scroll by points: the core shows the whole lines, the
    /// text slides by the rest. False for a terminal (its shell scrolls).
    private func slide(_ e: NSEvent) -> Bool {
        guard e.hasPreciseScrollingDeltas, let f = current else { return false }
        let (c, r) = cellAt(e)
        guard let w = f.windows.first(where: { c >= $0.x && c < $0.x + $0.w && r >= $0.y && r < $0.y + $0.h }),
              w.term != true, let id = w.id, let top = w.top, let lines = w.lines else { return false }
        var s = smooth?.win == id ? smooth! : (win: id, px: CGFloat(top) * cell.height, sent: top, seq: 0)
        s.px = min(max(0, s.px - e.scrollingDeltaY), CGFloat(maxTop(w, top: top, lines: lines)) * cell.height)
        let want = Int(s.px / cell.height)
        if want != s.sent {
            s.sent = want
            s.seq = transmit { .scroll(seq: $0, win: id, top: want) }
        }
        smooth = s
        redrawText(w)
        return true
    }

    /// The last top the core takes (its `scroll` clamp); one already past it stays.
    private func maxTop(_ w: Win, top: Int, lines: Int) -> Int { max(top, lines - visibleRows(w)) }

    /// Text rows on screen: the frame's last row is the next line, under the status bar, unless it is draw mode's toolbar.
    private func visibleRows(_ w: Win) -> Int {
        min(w.rows.count, max(0, w.h - (w.header ?? 0) - (w.toolbar == true ? 0 : 1)))
    }

    private func textRect(_ w: Win) -> NSRect {
        let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
        return NSRect(x: l, y: CGFloat(textY(w)) * cell.height, width: r - l, height: CGFloat(visibleRows(w)) * cell.height + grow(w))
    }

    /// Points left under the last whole row. The bottom status rows and the
    /// command line drop by it to sit flush with the edge; the windows above
    /// them grow into it, showing part of their next line.
    private var slack: CGFloat { current.map { max(0, bounds.height - CGFloat($0.rows) * cell.height) } ?? 0 }
    private func grow(_ w: Win) -> CGFloat { current.map { w.y + w.h == bottomTop($0) ? slack : 0 } ?? 0 }
    private func lowered(_ y: Int) -> Bool { current.map { y >= bottomTop($0) - 1 } ?? false }

    /// How far window `w`'s text is drawn moved down: the rubber band plus the slide.
    private func offset(_ w: Win) -> CGFloat {
        var dy = bounce?.win == w.id ? bounce!.dy : 0
        if let s = smooth, s.win == w.id, let top = w.top {
            // A frame behind the scroll slides at most a line, not off into nothing.
            dy += min(cell.height, max(-cell.height, CGFloat(top) * cell.height - s.px))
        }
        return dy
    }

    private func springBack() {
        guard bounce != nil, spring == nil else { return }
        springAt = CACurrentMediaTime()
        let link = displayLink(target: self, selector: #selector(springTick(_:)))
        link.add(to: .main, forMode: .common)
        spring = link
    }

    /// One display frame of the spring back, decayed by the real time elapsed.
    @objc private func springTick(_ link: CADisplayLink) {
        let now = link.targetTimestamp
        let dt = CGFloat(max(0, min(0.1, now - springAt)))
        springAt = now
        guard let b = bounce else { stopSpring(); return }
        let dy = Rubber.settle(b.dy, dt: dt)
        bounce = dy == 0 ? nil : (b.win, dy)
        if let w = current?.windows.first(where: { $0.id == b.win }) { redrawText(w) }
        if dy == 0 { stopSpring() }
    }

    private func stopSpring() {
        spring?.invalidate()
        spring = nil
    }

    private func redrawText(_ w: Win) { setNeedsDisplay(textRect(w)) }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let paintTracking { removeTrackingArea(paintTracking) }
        let tracking = NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(tracking)
        paintTracking = tracking
    }

    override func mouseMoved(with event: NSEvent) {
        if current?.windows.contains(where: { $0.paint != nil }) == true { needsDisplay = true }
    }

    override func mouseExited(with event: NSEvent) {
        if current?.windows.contains(where: { $0.paint != nil }) == true { needsDisplay = true }
    }

    // MARK: - Frames

    func apply(_ f: Frame) {
        let old = current
        current = f
        let menuOpen = f.leader != nil || !(f.bottom_paint?.isEmpty ?? true)
        let menuWasOpen = old?.leader != nil || !(old?.bottom_paint?.isEmpty ?? true)
        if f.leader != old?.leader { needsDisplay = true }
        if menuOpen && !menuWasOpen && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            panelSlideFrom = CACurrentMediaTime()
            panelSlideTimer?.invalidate()
            panelSlideTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) { [weak self] timer in
                guard self != nil else { timer.invalidate(); return }
                MainActor.assumeIsolated {
                    guard let self else { return }
                    guard let start = self.panelSlideFrom else { self.panelSlideTimer?.invalidate(); return }
                    self.needsDisplay = true
                    if CACurrentMediaTime() - start >= 0.12 {
                        self.panelSlideFrom = nil; self.panelSlideTimer?.invalidate()
                    }
                }
            }
        }
        if !menuOpen { panelSlideFrom = nil; panelSlideTimer?.invalidate() }
        blinkFrom = CACurrentMediaTime()
        blinkAlpha = 1
        layoutStatusBars(f)
        if old.map({ o in o.windows.count != f.windows.count || !zip(o.windows, f.windows).allSatisfy { sameRect($0, $1) } }) ?? true {
            window?.invalidateCursorRects(for: self)
        }
        layoutTabStrips(f)
        layoutMinimaps(f)
        layoutPicker(f, was: old?.palette != nil)
        // Scrolled some other way (keys, a click near the edge): the slide ends.
        if let s = smooth, f.seq >= s.seq {
            let w = f.windows.first { $0.id == s.win }
            if w?.top != s.sent {
                smooth = nil
                if let w { redrawText(w) }
            }
        }
        for w in f.windows where offset(w) != 0 { redrawText(w) }
        defer { if !needsDisplay { frameShown(f) } } // nothing to draw: the echo is already on screen
        guard let old, old.cols == f.cols, old.rows == f.rows, old.windows.count == f.windows.count,
              zip(old.windows, f.windows).allSatisfy({ sameRect($0, $1) })
        else { needsDisplay = true; return }
        for (o, n) in zip(old.windows, f.windows) {
            if o.paint != n.paint || o.paint_gutter != n.paint_gutter { redrawText(n) }
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

    /// Status lines and dividers between windows drag to resize (the core moves them).
    override func resetCursorRects() {
        guard let f = current, f.windows.count > 1 else { return }
        let bottom = bottomTop(f)
        for w in f.windows {
            let (l, r) = span(w.x, w.w)
            if w.y + w.h < bottom {
                addCursorRect(NSRect(x: l, y: CGFloat(w.y + w.h - 1) * cell.height, width: r - l, height: cell.height), cursor: .resizeUpDown)
            }
            if w.divider {
                let (dl, dr) = span(w.x + w.w - 1, 1)
                addCursorRect(NSRect(x: dl, y: CGFloat(w.y) * cell.height, width: dr - dl, height: CGFloat(w.h - 1) * cell.height), cursor: .resizeLeftRight)
            }
        }
    }

    /// One StatusBar per window, over its last cell row (minus the divider column).
    private func layoutStatusBars(_ f: Frame) {
        while statusBars.count < f.windows.count {
            let b = StatusBar(frame: .zero)
            b.editorBg = palette.bg
            addSubview(b, positioned: .below, relativeTo: overlay)
            statusBars.append(b)
        }
        for (i, b) in statusBars.enumerated() {
            guard i < f.windows.count else { b.isHidden = true; continue }
            let w = f.windows[i]
            let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
            b.frame = NSRect(x: l, y: CGFloat(w.y + w.h - 1) * cell.height + grow(w), width: r - l, height: cell.height)
            // Draw mode's toolbar takes the status row: the core draws it.
            b.isHidden = w.toolbar == true
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
            let c = Breadcrumbs(frame: .zero)
            c.editorBg = palette.bg
            c.reveal = { [weak self] in self?.reveal($0) }
            addSubview(c, positioned: .below, relativeTo: overlay)
            crumbs.append(c)
            let send = { [weak self] (op: String, arg: String?) in
                self?.transmit { .chat(seq: $0, op: op, arg: arg) }
                if let self { self.window?.makeFirstResponder(self) }
            }
            let h = ChatHeader(frame: .zero)
            h.editorBg = palette.bg
            h.send = send
            addSubview(h, positioned: .below, relativeTo: overlay)
            chatHeaders.append(h)
            let l = SessionsList(frame: .zero)
            l.editorBg = palette.bg
            l.send = send
            addSubview(l, positioned: .below, relativeTo: overlay)
            sessionLists.append(l)
        }
        for (i, s) in tabStrips.enumerated() {
            guard i < f.windows.count, case let w = f.windows[i], let id = w.id, let tabs = w.tabs, (w.header ?? 0) > 0 else {
                s.isHidden = true
                crumbs[i].isHidden = true
                chatHeaders[i].isHidden = true
                sessionLists[i].isHidden = true
                continue
            }
            let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
            // The chat: VS Code's Chat panel header in place of tabs and
            // breadcrumbs; before anything is said, the Sessions list
            // under its prompt line.
            if let chat = w.chat {
                s.isHidden = true
                crumbs[i].isHidden = true
                let h = chatHeaders[i]
                let hf = NSRect(x: l, y: CGFloat(w.y) * cell.height, width: r - l, height: CGFloat(w.header!) * cell.height)
                if h.frame != hf { h.frame = hf }
                h.isHidden = false
                h.update(chat)
                let list = sessionLists[i], sessions = chat.sessions ?? []
                list.isHidden = sessions.isEmpty
                let top = CGFloat(textY(w) + 2) * cell.height
                let lf = NSRect(x: l, y: top, width: r - l, height: max(0, CGFloat(w.y + w.h - 1) * cell.height - top))
                if list.frame != lf { list.frame = lf }
                list.update(sessions)
                continue
            }
            chatHeaders[i].isHidden = true
            sessionLists[i].isHidden = true
            // A header of 2+ rows ends with the breadcrumbs; one row is all tabs.
            let crumbRows = w.header! >= 2 ? 1 : 0
            let y = CGFloat(w.y) * cell.height, th = CGFloat(w.header! - crumbRows) * cell.height
            let f = NSRect(x: l, y: y, width: r - l, height: th)
            if s.frame != f { s.frame = f }
            s.isHidden = false
            let c = crumbs[i]
            c.isHidden = crumbRows == 0
            let cf = NSRect(x: l, y: y + th, width: r - l, height: CGFloat(crumbRows) * cell.height)
            if c.frame != cf { c.frame = cf }
            c.open = { [weak self] p in self?.transmit { .dropFile(seq: $0, path: p, target: id, zone: "center") } }
            c.symbols = { [weak self] in self?.transmit { .mac(seq: $0, key: "@", shift: false) } }
            c.update(path: w.path, root: root, symbol: w.symbol)
            s.op = { [weak self] op, index, to in self?.transmit { .tab(seq: $0, win: id, op: op, index: index, to: to) } }
            s.dragOut = { [weak self] index, p in
                guard let self else { return }
                let at = p.map { self.convert($0, from: nil) }
                self.showDrop(from: id, at)
                self.ghost.isHidden = at == nil
                if let at, index < tabs.count {
                    self.ghost.stringValue = "  \(tabs[index].name)  "
                    self.ghost.sizeToFit()
                    self.ghost.frame.origin = NSPoint(x: at.x + 12, y: at.y + 8)
                }
            }
            s.dropOut = { [weak self] index, p in
                guard let self, let d = self.dropTarget(from: id, self.convert(p, from: nil)) else { return }
                self.transmit { .drop(seq: $0, win: id, index: index, target: d.win, zone: d.zone, to: d.to) }
            }
            s.update(tabs, active: w.active)
        }
    }

    /// The palette floats at the top center of the editor, fading in when a picker opens.
    private func layoutPicker(_ f: Frame, was: Bool) {
        guard let p = f.palette else {
            if was {
                NSAnimationContext.runAnimationGroup({ $0.duration = 0.08; picker.animator().alphaValue = 0 },
                                                     completionHandler: { [picker] in MainActor.assumeIsolated { if picker.alphaValue == 0 { picker.isHidden = true } } })
                if let c = f.cursor { setNeedsDisplay(rowRect(c.y)) }
            }
            return
        }
        if picker.superview == nil {
            picker.editorBg = palette.bg
            picker.pick = { [weak self] i in self?.transmit { .pick(seq: $0, index: i) } }
            picker.scroll = { [weak self] n in
                let k = String(repeating: n > 0 ? "<Down>" : "<Up>", count: abs(n))
                self?.transmit { .keys(seq: $0, keys: k) }
            }
            addSubview(picker)
        }
        let w = min(800, bounds.width - 40) // room for command descriptions
        let top = CGFloat(f.windows.map(\.y).min() ?? 0) * cell.height + 6
        picker.frame = NSRect(x: (bounds.width - w) / 2, y: top, width: w, height: PaletteView.height(p))
        picker.update(p)
        if !was {
            if let c = f.cursor { setNeedsDisplay(rowRect(c.y)) }
            picker.isHidden = false
            picker.alphaValue = 0
            NSAnimationContext.runAnimationGroup { $0.duration = 0.1; picker.animator().alphaValue = 1 }
        }
    }

    // MARK: - Tab drag and drop

    private struct Drop { var win: Int; var zone: String; var to: Int; var hint: NSRect }

    /// Where a tab from window `from` dropped at `p` (view coordinates)
    /// goes, as VS Code: a tab strip inserts at its gap; an editor's middle
    /// moves it there, a third from an edge splits that way. Nil where it
    /// would change nothing.
    private func dropTarget(from: Int, _ p: NSPoint) -> Drop? {
        guard let f = current else { return nil }
        for (i, w) in f.windows.enumerated() {
            guard let id = w.id, let tabs = w.tabs else { continue }
            let (l, r) = span(w.x, w.w - (w.divider ? 1 : 0))
            let header = CGFloat(w.header ?? 0) * cell.height
            let full = NSRect(x: l, y: CGFloat(w.y) * cell.height, width: r - l, height: CGFloat(w.h - 1) * cell.height)
            guard full.contains(p) else { continue }
            if p.y < full.minY + header, i < tabStrips.count {
                let strip = tabStrips[i], gap = strip.slot(atX: p.x - strip.frame.minX)
                return Drop(win: id, zone: "center", to: gap.index,
                            hint: NSRect(x: strip.frame.minX + gap.x - 1, y: full.minY, width: 2, height: header))
            }
            let body = NSRect(x: full.minX, y: full.minY + header, width: full.width, height: full.height - header)
            let z = TabMath.dropZone(p, in: body)
            if id == from && (z.zone == "center" || tabs.count < 2) { return nil }
            return Drop(win: id, zone: z.zone, to: tabs.count, hint: z.hint)
        }
        return nil
    }

    /// The drop hint for a drag at `p` (view coordinates); nil fades it
    /// out. As VS Code's, it fades in and slides between zones.
    private func showDrop(from: Int, _ p: NSPoint?) {
        guard let p, let d = dropTarget(from: from, p) else {
            guard !dropHint.isHidden, hintShown else { return }
            hintShown = false
            NSAnimationContext.runAnimationGroup({ c in
                c.duration = 0.1
                dropHint.animator().alphaValue = 0
            }, completionHandler: { [weak self] in
                MainActor.assumeIsolated { if self?.hintShown == false { self?.dropHint.isHidden = true } }
            })
            return
        }
        if !hintShown {
            hintShown = true
            if dropHint.isHidden || dropHint.alphaValue == 0 { dropHint.frame = d.hint }
            dropHint.isHidden = false
            NSAnimationContext.runAnimationGroup { c in
                c.duration = 0.1
                dropHint.animator().alphaValue = 1
                dropHint.animator().frame = d.hint
            }
        } else if dropHint.frame != d.hint {
            NSAnimationContext.runAnimationGroup { c in
                c.duration = 0.15
                c.timingFunction = CAMediaTimingFunction(name: .easeOut)
                dropHint.animator().frame = d.hint
            }
        }
    }

    /// Files and folders (as dired) dragged in from the sidebar or Finder: any window, any zone.
    private func fileURLs(_ s: NSDraggingInfo) -> [URL] {
        s.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
    }

    override func draggingEntered(_ s: NSDraggingInfo) -> NSDragOperation { draggingUpdated(s) }

    override func draggingUpdated(_ s: NSDraggingInfo) -> NSDragOperation {
        guard !fileURLs(s).isEmpty else { return [] }
        let p = convert(s.draggingLocation, from: nil)
        showDrop(from: -1, p)
        return dropTarget(from: -1, p) == nil ? [] : .copy
    }

    override func draggingExited(_ s: NSDraggingInfo?) { showDrop(from: -1, nil) }

    override func performDragOperation(_ s: NSDraggingInfo) -> Bool {
        showDrop(from: -1, nil)
        let urls = fileURLs(s)
        guard let first = urls.first, let d = dropTarget(from: -1, convert(s.draggingLocation, from: nil)) else { return false }
        transmit { .dropFile(seq: $0, path: first.path, target: d.win, zone: d.zone) }
        // The rest join the window the first landed in, now the active one.
        for u in urls.dropFirst() { transmit { .open(seq: $0, path: u.path, preview: false) } }
        window?.makeFirstResponder(self)
        return true
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
            m.update(maps[id], top: w.top ?? 0, visible: visible, lines: w.lines)
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

    /// Row `y`'s band; a lowered row's also covers the slack above where it is drawn.
    private func rowRect(_ y: Int) -> NSRect {
        NSRect(x: 0, y: CGFloat(y) * cell.height, width: bounds.width, height: cell.height + (lowered(y) ? slack : 0))
    }

    // MARK: - Drawing

    override func draw(_ dirty: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        // Glass: the box behind us paints the tint, so it isn't doubled here.
        if palette.clear { ctx.clear(dirty) } else { ctx.setFillColor(palette.bg.cgColor); ctx.fill(dirty) }
        guard let f = current else { return }
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        for w in f.windows {
            let y = textY(w), vis = visibleRows(w)
            // Clipped to its text rows; a slid or stretched window moves all its rows.
            let dy = offset(w)
            ctx.saveGState()
            ctx.clip(to: textRect(w))
            if dy != 0 { ctx.translateBy(x: 0, y: dy) }
            let dirty = dy != 0 ? Array(w.rows.indices) : w.rows.indices.filter { $0 < vis + (grow(w) > 0 ? 1 : 0) && needsToDraw(rowRect(y + $0)) }
            // VS Code's current line: a faint band in the active window, without a selection.
            if w.active, w.sel?.isEmpty ?? true, let c = f.cursor, case let r = c.y - y, dirty.contains(r) {
                let (l, rr) = span(w.x, w.w - (w.divider ? 1 : 0))
                ctx.setFillColor(palette.fg.withAlphaComponent(0.045).cgColor)
                ctx.fill(CGRect(x: l, y: CGFloat(c.y) * cell.height, width: rr - l, height: cell.height))
            }
            for r in dirty {
                let role = w.paint?.indices.contains(r) == true ? w.paint?[r].role?.kind : nil
                let runs = w.rows[r].map { run -> Run in
                    var run = run
                    // Replace only the diff base fills, keeping whitespace errors
                    // and refined-word backgrounds visible.
                    if (role == "added" && run.bg == 22) || (role == "removed" && run.bg == 52) { run.bg = nil }
                    return run
                }
                drawBackgrounds(ctx, runs, col: w.x, row: y + r)
            }
            drawPaint(ctx, w)
            if let sel = w.sel, !sel.isEmpty { drawSelection(ctx, sel, w) }
            for r in dirty {
                let paint = w.paint?.indices.contains(r) == true ? w.paint?[r] : nil
                drawText(ctx, paintedRuns(w.rows[r], paint), col: w.x, row: y + r)
            }
            if dy != 0, w.active, f.palette == nil, let c = f.cursor, c.y >= y, c.y < y + vis { drawCursor(ctx, c, f) }
            ctx.restoreGState()
            if w.divider {
                let x = snap((CGFloat(w.x + w.w - 1) + 0.5) * cell.width, scale: scale)
                ctx.setFillColor(NSColor.separatorColor.cgColor)
                ctx.fill(CGRect(x: x, y: CGFloat(w.y) * cell.height, width: 1 / scale,
                                height: CGFloat(w.h) * cell.height + grow(w)))
            }
        }
        let top = bottomTop(f)
        ctx.saveGState()
        ctx.translateBy(x: 0, y: slack)
        if let start = panelSlideFrom, !(f.bottom_paint?.isEmpty ?? true) {
            let progress = min(1, (CACurrentMediaTime() - start) / 0.12)
            ctx.clip(to: CGRect(x: 0, y: CGFloat(top) * cell.height, width: bounds.width, height: CGFloat(f.bottom.count) * cell.height))
            ctx.translateBy(x: 0, y: cell.height * 0.6 * CGFloat((1 - progress) * (1 - progress)))
        }
        for (r, runs) in f.bottom.enumerated() where needsToDraw(rowRect(top + r)) {
            drawBackgrounds(ctx, runs, col: 0, row: top + r)
        }
        drawPanelPaint(ctx, f)
        for (r, runs) in f.bottom.enumerated() where needsToDraw(rowRect(top + r)) {
            let paint = f.bottom_paint?.indices.contains(r) == true ? f.bottom_paint?[r] : nil
            drawText(ctx, paintedRuns(runs, paint), col: 0, row: top + r)
        }
        ctx.restoreGState()
        if let menu = f.leader {
            ctx.saveGState()
            let rect = CGRect(x: CGFloat(menu.x) * cell.width, y: CGFloat(menu.y) * cell.height,
                              width: CGFloat(menu.w) * cell.width, height: CGFloat(menu.rows.count) * cell.height)
            ctx.clip(to: rect)
            if let start = panelSlideFrom {
                let progress = min(1, (CACurrentMediaTime() - start) / 0.12)
                ctx.translateBy(x: 0, y: cell.height * 0.6 * CGFloat((1 - progress) * (1 - progress)))
            }
            ctx.setFillColor(palette.bg.cgColor)
            ctx.fill(rect)
            for (r, runs) in menu.rows.enumerated() {
                drawBackgrounds(ctx, runs, col: menu.x, row: menu.y + r)
                drawText(ctx, runs, col: menu.x, row: menu.y + r)
            }
            ctx.restoreGState()
        }
        // With the palette open, its own caret is the cursor.
        // A slid or stretched window drew its own cursor.
        var stretched = false
        if let c = f.cursor, let w = f.windows.first(where: { c.y >= textY($0) && c.y < textY($0) + visibleRows($0) && c.x >= $0.x && c.x < $0.x + $0.w }) {
            stretched = offset(w) != 0
        }
        if f.palette == nil, !stretched, let c = f.cursor, needsToDraw(rowRect(c.y)) { drawCursor(ctx, c, f) }
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
            if run.s.unicodeScalars.contains(where: { (0x2580...0x2590).contains($0.value) }) {
                var at = col + run.x
                for ch in run.s {
                    let s = String(ch)
                    if let b = block(ch, at: at, row: row) {
                        ctx.setFillColor(colors(run).fg.cgColor)
                        ctx.fill(b)
                    } else {
                        ctx.textPosition = CGPoint(x: span(at, 1).0, y: base)
                        CTLineDraw(line(key(s)), ctx)
                    }
                    at += cellWidth(s)
                }
            } else if cellWidth(run.s) == run.s.count {
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

    /// Block elements (▀ ▄ █ ▌ ▐, eighths) as cell rectangles: their glyphs
    /// leave gaps in rows taller than the font, which breaks the splash logo.
    private func block(_ ch: Character, at col: Int, row: Int) -> CGRect? {
        guard ch.unicodeScalars.count == 1, let v = ch.unicodeScalars.first?.value, (0x2580...0x2590).contains(v) else { return nil }
        let (l, r) = span(col, 1)
        let top = CGFloat(row) * cell.height, h = cell.height
        switch v {
        case 0x2580: // upper half
            return CGRect(x: l, y: top, width: r - l, height: snap(top + h / 2, scale: scale) - top)
        case 0x2581...0x2588: // lower n eighths, to full
            let t = snap(top + h * (1 - CGFloat(v - 0x2580) / 8), scale: scale)
            return CGRect(x: l, y: t, width: r - l, height: top + h - t)
        case 0x2589...0x258F: // left n eighths
            return CGRect(x: l, y: top, width: snap(l + (r - l) * CGFloat(0x2590 - v) / 8, scale: scale) - l, height: h)
        default: // right half
            let m = snap((l + r) / 2, scale: scale)
            return CGRect(x: m, y: top, width: r - m, height: h)
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
        ctx.saveGState()
        defer { ctx.restoreGState() }
        ctx.setAlpha(blinkAlpha)
        if lowered(c.y) { ctx.translateBy(x: 0, y: slack) }
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
