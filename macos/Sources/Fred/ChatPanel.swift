import AppKit
import FredKit

/// VS Code's Chat panel header over the chat window's header rows: the
/// title, new chat and close, then the model and permission mode.
final class ChatHeader: NSView {
    var send: (_ op: String, _ arg: String?) -> Void = { _, _ in }
    var editorBg = NSColor.black { didSet { needsDisplay = true } }

    private let title = NSTextField(labelWithString: "Chat")
    private let model = Pill(title: "")
    private let mode = Pill(title: "")
    private let status = NSTextField(labelWithString: "")

    override init(frame: NSRect) {
        super.init(frame: frame)
        title.font = .systemFont(ofSize: 13, weight: .semibold)
        status.font = .systemFont(ofSize: 11)
        status.textColor = .secondaryLabelColor
        let new = ChatHeader.icon("plus", "New Chat") { [unowned self] in self.send("new", nil) }
        let close = ChatHeader.icon("xmark", "Close Chat") { [unowned self] in self.send("toggle", nil) }
        model.onClick = { [unowned self] in self.send("model", nil) }
        mode.onClick = { [unowned self] in self.send("mode", nil) }
        model.toolTip = "Model"
        mode.toolTip = "Permission mode"
        for v in [title, new, close, model, mode, status] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            title.centerYAnchor.constraint(equalTo: topAnchor, constant: 13),
            close.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            close.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            new.trailingAnchor.constraint(equalTo: close.leadingAnchor, constant: -4),
            new.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            model.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            model.centerYAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            mode.leadingAnchor.constraint(equalTo: model.trailingAnchor, constant: 4),
            mode.centerYAnchor.constraint(equalTo: model.centerYAnchor),
            status.leadingAnchor.constraint(equalTo: mode.trailingAnchor, constant: 8),
            status.centerYAnchor.constraint(equalTo: model.centerYAnchor),
            status.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func draw(_ r: NSRect) {
        editorBg.setFill()
        bounds.fill()
        NSColor.separatorColor.setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
    }

    func update(_ c: ChatBar) {
        model.title = "✳ " + (c.model.map(ChatHeader.modelName) ?? "Claude")
        mode.title = "◈ " + ChatHeader.modeName(c.mode)
        status.stringValue = c.busy ? "Working…" : ""
    }

    /// `claude-opus-5-5` → `Opus 5.5`; an alias (`opus`) capitalized.
    static func modelName(_ id: String) -> String {
        let parts = id.split(separator: "-").filter { $0 != "claude" }
        guard let name = parts.first else { return id }
        let version = parts.dropFirst().prefix { $0.count < 3 && $0.allSatisfy(\.isNumber) }
        return name.capitalized + (version.isEmpty ? "" : " " + version.joined(separator: "."))
    }

    /// Claude Code's permission modes, as VS Code's chat names them.
    static func modeName(_ mode: String?) -> String {
        switch mode {
        case "acceptEdits": return "Edit Automatically"
        case "plan": return "Plan Mode"
        case "auto": return "Auto Mode"
        case "dontAsk": return "Don't Ask"
        case "bypassPermissions": return "Bypass Permissions"
        default: return "Ask Before Edits"
        }
    }

    private static func icon(_ symbol: String, _ tip: String, _ run: @escaping () -> Void) -> NSView {
        let b = Pill(title: "")
        b.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
        b.toolTip = tip
        b.onClick = run
        b.square = true
        return b
    }
}

/// A borderless button: text or an icon, a faint fill on hover.
final class Pill: NSView {
    var title: String { didSet { invalidateIntrinsicContentSize(); needsDisplay = true } }
    var image: NSImage?
    var square = false
    /// An icon drawn bright, for a part of the layout that is showing.
    var lit = false { didSet { if lit != oldValue { needsDisplay = true } } }
    /// A lit filled symbol in the accent colour (the chat's send button).
    var accent = false
    var onClick: () -> Void = {}
    private var hover = false
    private static let font = NSFont.systemFont(ofSize: 12)

    init(title: String) {
        self.title = title
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override var intrinsicContentSize: NSSize {
        square ? NSSize(width: 22, height: 22)
            : NSSize(width: (title as NSString).size(withAttributes: [.font: Pill.font]).width + 14, height: 20)
    }

    override func draw(_ r: NSRect) {
        if accent && image == nil {
            NSColor.controlAccentColor.withAlphaComponent(hover ? 1 : 0.85).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()
        } else if hover {
            NSColor.white.withAlphaComponent(0.08).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()
        }
        if let image {
            let cfg = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
                .applying(.init(paletteColors: accent && lit ? [.white, .controlAccentColor] : [lit ? .labelColor : .secondaryLabelColor]))
            let i = image.withSymbolConfiguration(cfg) ?? image
            let s = i.size
            i.draw(in: NSRect(x: (bounds.width - s.width) / 2, y: (bounds.height - s.height) / 2, width: s.width, height: s.height),
                   from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        } else {
            let t = NSAttributedString(string: title, attributes: [.font: Pill.font, .foregroundColor: accent ? NSColor.white : NSColor.secondaryLabelColor])
            t.draw(at: NSPoint(x: 7, y: (bounds.height - t.size().height) / 2))
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with e: NSEvent) { hover = true; needsDisplay = true }
    override func mouseExited(with e: NSEvent) { hover = false; needsDisplay = true }
    override func mouseDown(with e: NSEvent) {}
    override func mouseUp(with e: NSEvent) { if bounds.contains(convert(e.locationInWindow, from: nil)) { onClick() } }
}

/// VS Code's Sessions list, shown in a chat that hasn't started: past
/// conversations, newest first; a click goes on with one.
final class SessionsList: NSView {
    var send: (_ op: String, _ arg: String?) -> Void = { _, _ in }
    var editorBg = NSColor.black { didSet { needsDisplay = true } }

    /// Collapsed to its heading row over the blank line above the composer;
    /// open, it grows upward over the chat.
    var open = false
    var onToggle: () -> Void = {}
    /// The heading row's height: one text row.
    var headH: CGFloat = 20

    private var rows: [[String]] = []
    private var hover: Int?
    private static let rowH: CGFloat = 44, moreH: CGFloat = 28, most = 6

    /// Its height open, all of its first `most` rows showing.
    var openHeight: CGFloat { headH + CGFloat(min(rows.count, Self.most)) * Self.rowH + Self.moreH + 4 }

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(_ sessions: [[String]]) {
        guard sessions != rows else { return }
        rows = sessions
        hover = nil
        needsDisplay = true
    }

    /// Rows that fit; the rest are behind More.
    private var shown: Int { open ? max(0, min(rows.count, Self.most, Int((bounds.height - headH - Self.moreH) / Self.rowH))) : 0 }

    private var headRect: NSRect { NSRect(x: 6, y: 0, width: bounds.width - 12, height: headH) }

    private func rowRect(_ i: Int) -> NSRect {
        NSRect(x: 6, y: headH + CGFloat(i) * Self.rowH, width: bounds.width - 12, height: Self.rowH)
    }

    private var moreRect: NSRect {
        NSRect(x: 6, y: headH + CGFloat(shown) * Self.rowH, width: bounds.width - 12, height: open ? Self.moreH : 0)
    }

    override func draw(_ r: NSRect) {
        editorBg.setFill()
        bounds.fill()
        if open {
            let card = NSBezierPath(roundedRect: bounds.insetBy(dx: 4.5, dy: 0.5), xRadius: 6, yRadius: 6)
            NSColor.white.withAlphaComponent(0.04).setFill()
            card.fill()
            NSColor.white.withAlphaComponent(0.12).setStroke()
            card.stroke()
        }
        if hover == -2 {
            NSColor.white.withAlphaComponent(0.06).setFill()
            NSBezierPath(roundedRect: headRect, xRadius: 5, yRadius: 5).fill()
        }
        let head = NSAttributedString(string: (open ? "⌄  " : "›  ") + "Sessions", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.secondaryLabelColor])
        head.draw(at: NSPoint(x: 12, y: (headH - head.size().height) / 2))
        if !open {
            let n = NSAttributedString(string: "\(rows.count)", attributes: [
                .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.tertiaryLabelColor])
            n.draw(at: NSPoint(x: headRect.maxX - 10 - n.size().width, y: (headH - n.size().height) / 2))
            return
        }
        let clip = NSMutableParagraphStyle()
        clip.lineBreakMode = .byTruncatingTail
        for i in 0..<shown {
            let f = rowRect(i), row = rows[i]
            if hover == i {
                NSColor.white.withAlphaComponent(0.06).setFill()
                NSBezierPath(roundedRect: f, xRadius: 5, yRadius: 5).fill()
            }
            let title = NSAttributedString(string: row.count > 1 ? row[1] : "", attributes: [
                .font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.labelColor, .paragraphStyle: clip])
            title.draw(in: NSRect(x: f.minX + 10, y: f.minY + 5, width: f.width - 20, height: 18))
            let ago = NSAttributedString(string: "✳ · " + (row.count > 2 ? row[2] : ""), attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.tertiaryLabelColor])
            ago.draw(at: NSPoint(x: f.minX + 10, y: f.minY + 24))
        }
        let more = NSAttributedString(string: "More", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold),
            .foregroundColor: hover == -1 ? NSColor.labelColor : NSColor.secondaryLabelColor])
        more.draw(at: NSPoint(x: moreRect.minX + 10, y: moreRect.minY + 6))
        let count = NSAttributedString(string: "\(rows.count)", attributes: [
            .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.tertiaryLabelColor])
        count.draw(at: NSPoint(x: moreRect.maxX - 10 - count.size().width, y: moreRect.minY + 6))
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    /// The row under `e`: its index, -1 for More, -2 the heading, nil for none.
    private func index(_ e: NSEvent) -> Int? {
        let p = convert(e.locationInWindow, from: nil)
        if headRect.contains(p) { return -2 }
        if moreRect.contains(p) { return -1 }
        return (0..<shown).first { rowRect($0).contains(p) }
    }

    override func mouseMoved(with e: NSEvent) {
        let h = index(e)
        if h != hover { hover = h; needsDisplay = true }
    }

    override func mouseExited(with e: NSEvent) { hover = nil; needsDisplay = true }
    override func mouseDown(with e: NSEvent) {}

    override func mouseUp(with e: NSEvent) {
        guard let i = index(e) else { return }
        if rows.indices.contains(i) { send("resume", rows[i].first) } // -2 heading, -1 More
        if i == -1 { send("more", nil) }
        open = i == -2 && !open
        needsDisplay = true
        onToggle()
    }
}

/// The chat's text drawn as VS Code's: your messages in bubbles, tool
/// calls as cards, the prompt in a composer box (core sends row kinds).
extension EditorView {
    func drawChat(_ ctx: CGContext, _ w: Win) {
        guard let kinds = w.chat?.rows.map(Array.init), !kinds.isEmpty else { return }
        let y = w.y + (w.header ?? 0)
        let width = w.w - (w.margin ?? 0) - (w.divider ? 1 : 0)
        // A tool call and its result are one card.
        let group: (Character) -> Character = { $0 == "r" ? "t" : $0 }
        var r = 0
        while r < kinds.count {
            let k = group(kinds[r])
            var end = r + 1
            while end < kinds.count, group(kinds[end]) == k { end += 1 }
            // The question's card holds the Allow / Deny row under it.
            if k == "a", end < kinds.count, kinds[end] == " " { end += 1 }
            var box = paintRect(w.x, y + r, width).insetBy(dx: cell.width / 2, dy: 0)
            box.size.height = CGFloat(end - r) * cell.height
            switch k {
            case "u":
                paintFill(ctx, box.insetBy(dx: 0, dy: -3), palette.fg.withAlphaComponent(0.07), radius: 6)
            case "a":
                let card = box.insetBy(dx: 0, dy: -2)
                paintFill(ctx, card, NSColor.systemOrange.withAlphaComponent(0.08), radius: 5)
                paintFill(ctx, CGRect(x: card.minX, y: card.minY + 3, width: 2, height: card.height - 6), NSColor.systemOrange.withAlphaComponent(0.8), radius: 1)
            case "t":
                let card = box.insetBy(dx: 0, dy: -2)
                paintFill(ctx, card, palette.fg.withAlphaComponent(0.035), radius: 5)
                paintFill(ctx, CGRect(x: card.minX, y: card.minY + 3, width: 2, height: card.height - 6), NSColor.controlAccentColor.withAlphaComponent(0.6), radius: 1)
            case "p":
                let c = box.insetBy(dx: 0, dy: -4)
                paintFill(ctx, c, palette.fg.withAlphaComponent(0.04), radius: 8)
                ctx.setStrokeColor((w.active ? NSColor.controlAccentColor.withAlphaComponent(0.8) : palette.fg.withAlphaComponent(0.18)).cgColor)
                ctx.setLineWidth(1)
                ctx.addPath(CGPath(roundedRect: c.insetBy(dx: 0.5, dy: 0.5), cornerWidth: 8, cornerHeight: 8, transform: nil))
                ctx.strokePath()
                // Nothing typed yet: VS Code's placeholder after the `>`.
                if end - r == 1, w.rows[r].map(\.s).joined().trimmingCharacters(in: .whitespaces) == ">" {
                    let busy = w.chat?.busy == true
                    let hint = NSAttributedString(string: busy ? "Claude is working — C-c stops" : "Ask Claude — Enter sends, ⇧Enter for a new line", attributes: [
                        .font: font, .foregroundColor: palette.fg.withAlphaComponent(0.35)])
                    // AppKit's text drawing changes the text matrix, which isn't part of the saved state.
                    let m = ctx.textMatrix, x = box.minX + cell.width * 2.7
                    ctx.saveGState()
                    ctx.clip(to: CGRect(x: x, y: box.minY, width: max(0, box.maxX - (busy ? 54 : 30) - x), height: box.height)) // clear of the spinner and send button
                    hint.draw(at: NSPoint(x: x, y: box.minY + (cell.height - hint.size().height) / 2))
                    ctx.restoreGState()
                    ctx.textMatrix = m
                }
            default: break
            }
            r = end
        }
    }
}
