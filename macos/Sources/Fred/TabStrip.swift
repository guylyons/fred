import AppKit
import FredKit

/// Sublime's tab strip over a window's header row: rounded-top tabs, the
/// current one in the editor background so it joins the text.
final class TabStrip: NSView {
    /// (op, index, to): a `tab` message for this strip's window.
    var op: (String, Int, Int?) -> Void = { _, _, _ in }
    /// Reveal in Sidebar.
    var reveal: (String) -> Void = { _ in }
    var editorBg = NSColor.black

    private var tabs: [Tab] = []
    /// Natural widths, measured once per tabs change.
    private var widths: [CGFloat] = []
    private var frames: [CGRect] = []
    private var offset: CGFloat = 0
    private var hover: Int?
    private var hoverClose = false
    private var press: (index: Int, x: CGFloat, onClose: Bool)?
    private var dragDX: CGFloat?

    private static let font = NSFont.systemFont(ofSize: 12)
    private static let italic: NSFont = {
        let f = NSFont.systemFont(ofSize: 12)
        return NSFont(descriptor: f.fontDescriptor.withSymbolicTraits(.italic), size: 12) ?? f
    }()
    private static let pad: CGFloat = 12, closeW: CGFloat = 16

    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true // else the dirty rect can reach past us, over the text
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(_ tabs: [Tab], active: Bool) {
        alphaValue = active ? 1 : 0.7
        guard tabs != self.tabs else { return }
        // A press or hover on a tab that moved or went away would index past the end.
        if tabs.count != self.tabs.count || press.map({ tabs[$0.index].name != self.tabs[$0.index].name }) == true {
            press = nil
            dragDX = nil
            hover = nil
        }
        self.tabs = tabs
        widths = tabs.map { name($0).size().width + 2 * TabStrip.pad + TabStrip.closeW }
        relayout()
    }

    /// EditorView sets our frame every core frame: only a new size relays out.
    override func setFrameSize(_ s: NSSize) {
        guard s != frame.size else { return }
        super.setFrameSize(s)
        relayout()
    }

    private func relayout() {
        let cur = tabs.firstIndex(where: \.current) ?? 0
        let l = TabMath.layout(widths: widths, available: bounds.width, current: cur, offset: offset)
        frames = l.frames
        if press == nil { offset = l.offset } // a pressed or dragged tab doesn't jump
        needsDisplay = true
    }

    private func name(_ t: Tab) -> NSAttributedString {
        let p = NSMutableParagraphStyle()
        p.lineBreakMode = .byTruncatingTail
        return NSAttributedString(string: t.name, attributes: [
            .font: t.preview ? TabStrip.italic : TabStrip.font,
            .foregroundColor: t.current ? NSColor.labelColor : NSColor.secondaryLabelColor,
            .paragraphStyle: p,
        ])
    }

    /// Tab `i` on screen (content frame minus the scroll, full height).
    private func rect(_ i: Int) -> CGRect {
        var r = frames[i].offsetBy(dx: -offset, dy: 0)
        r.origin.y = 0
        r.size.height = bounds.height
        if i == press?.index, let dx = dragDX { r.origin.x += dx }
        return r
    }

    private func closeRect(_ r: CGRect) -> CGRect {
        CGRect(x: r.maxX - TabStrip.pad / 2 - TabStrip.closeW, y: r.midY - TabStrip.closeW / 2 + 1,
               width: TabStrip.closeW, height: TabStrip.closeW)
    }

    // MARK: - Drawing

    override func draw(_ dirty: NSRect) {
        editorBg.blended(withFraction: 0.06, of: .black)!.setFill()
        bounds.fill()
        // The dragged tab draws last, over its neighbours.
        let order = tabs.indices.filter { $0 != press?.index || dragDX == nil } + (dragDX == nil ? [] : [press!.index])
        for i in order { drawTab(i) }
    }

    private func drawTab(_ i: Int) {
        let t = tabs[i], r = rect(i)
        if t.current || (i == press?.index && dragDX != nil) {
            editorBg.setFill()
            topRounded(r.divided(atDistance: 3, from: .minYEdge).remainder, radius: 6).fill()
        } else if i == hover {
            NSColor.white.withAlphaComponent(0.04).setFill()
            topRounded(r.divided(atDistance: 3, from: .minYEdge).remainder, radius: 6).fill()
        }
        let s = name(t), h = s.size().height
        let close = closeRect(r)
        s.draw(with: CGRect(x: r.minX + TabStrip.pad, y: r.midY - h / 2 + 1,
                            width: close.minX - r.minX - TabStrip.pad - 2, height: h),
               options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        // ● for modified, × on hover (a hovered ● turns into ×).
        let glyph = i == hover ? "×" : t.modified ? "●" : nil
        guard let glyph else { return }
        if i == hover, hoverClose {
            NSColor.white.withAlphaComponent(0.12).setFill()
            NSBezierPath(roundedRect: close, xRadius: 3, yRadius: 3).fill()
        }
        let g = NSAttributedString(string: glyph, attributes: [
            .font: NSFont.systemFont(ofSize: glyph == "●" ? 9 : 13),
            .foregroundColor: t.current ? NSColor.labelColor : NSColor.secondaryLabelColor,
        ])
        let gs = g.size()
        g.draw(at: NSPoint(x: close.midX - gs.width / 2, y: close.midY - gs.height / 2))
    }

    private func topRounded(_ r: CGRect, radius: CGFloat) -> NSBezierPath {
        let p = NSBezierPath()
        p.move(to: NSPoint(x: r.minX, y: r.maxY))
        p.line(to: NSPoint(x: r.minX, y: r.minY + radius))
        p.appendArc(withCenter: NSPoint(x: r.minX + radius, y: r.minY + radius), radius: radius, startAngle: 180, endAngle: 270)
        p.line(to: NSPoint(x: r.maxX - radius, y: r.minY))
        p.appendArc(withCenter: NSPoint(x: r.maxX - radius, y: r.minY + radius), radius: radius, startAngle: 270, endAngle: 360)
        p.line(to: NSPoint(x: r.maxX, y: r.maxY))
        p.close()
        return p
    }

    // MARK: - Mouse

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self))
    }

    private func point(_ e: NSEvent) -> CGPoint { convert(e.locationInWindow, from: nil) }

    private func index(at p: CGPoint) -> Int? {
        tabs.indices.first { rect($0).contains(p) }
    }

    override func mouseMoved(with e: NSEvent) { setHover(point(e)) }
    override func mouseExited(with e: NSEvent) { setHover(nil) }

    private func setHover(_ p: CGPoint?) {
        let h = p.flatMap(index(at:)), c = h.map { closeRect(rect($0)).contains(p!) } ?? false
        guard h != hover || c != hoverClose else { return }
        hover = h
        hoverClose = c
        needsDisplay = true
    }

    /// Empty strip space swallows right-clicks (they'd reach the core as a
    /// click on the header row); a tab gets its menu.
    override func rightMouseDown(with e: NSEvent) {
        if let m = menu(for: e) { NSMenu.popUpContextMenu(m, with: e, for: self) }
    }

    override func mouseDown(with e: NSEvent) {
        if e.modifierFlags.contains(.control) { rightMouseDown(with: e); return }
        let p = point(e)
        guard let i = index(at: p) else { return }
        let onClose = closeRect(rect(i)).contains(p)
        press = (i, p.x, onClose)
        if onClose { return }
        if e.clickCount == 2, tabs[i].preview { op("keep", i, nil) } else if !tabs[i].current { op("select", i, nil) }
    }

    override func mouseDragged(with e: NSEvent) {
        guard let pr = press, !pr.onClose else { return }
        let dx = point(e).x - pr.x
        guard dragDX != nil || abs(dx) > 4 else { return }
        dragDX = dx
        needsDisplay = true
    }

    override func mouseUp(with e: NSEvent) {
        defer { press = nil; dragDX = nil; setHover(point(e)) }
        guard let pr = press else { return }
        if pr.onClose {
            if index(at: point(e)) == pr.index, closeRect(rect(pr.index)).contains(point(e)) { op("close", pr.index, nil) }
        } else if dragDX != nil {
            // The last slot whose left edge the dragged tab's middle has passed.
            let mid = rect(pr.index).midX + offset
            let to = frames.lastIndex { $0.minX <= mid } ?? 0
            if to != pr.index { op("move", pr.index, to) }
        }
    }

    override func otherMouseDown(with e: NSEvent) {
        if e.buttonNumber == 2, let i = index(at: point(e)) { op("close", i, nil) }
    }

    override func menu(for e: NSEvent) -> NSMenu? {
        guard let i = index(at: point(e)) else { return nil }
        let m = NSMenu()
        m.autoenablesItems = false
        func add(_ title: String, enabled: Bool = true, _ act: @escaping () -> Void) {
            let item = m.addItem(withTitle: title, action: #selector(MenuAction.run(_:)), keyEquivalent: "")
            let a = MenuAction(act)
            item.target = a
            item.representedObject = a // keeps it alive
            item.isEnabled = enabled
        }
        let n = tabs.count, path = tabs[i].path
        add("Close") { self.op("close", i, nil) }
        // Right to left, so the indices still to send stay valid.
        add("Close Others", enabled: n > 1) { for j in (0..<n).reversed() where j != i { self.op("close", j, nil) } }
        add("Close Tabs to the Right", enabled: i < n - 1) { for j in ((i + 1)..<n).reversed() { self.op("close", j, nil) } }
        m.addItem(.separator())
        add("Copy Path", enabled: path != nil) {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(path!, forType: .string)
        }
        add("Reveal in Sidebar", enabled: path != nil) { self.reveal(path!) }
        return m
    }
}

/// A closure as a menu item target.
private final class MenuAction: NSObject {
    let body: () -> Void
    init(_ body: @escaping () -> Void) { self.body = body }
    @objc func run(_ sender: Any?) { body() }
}
