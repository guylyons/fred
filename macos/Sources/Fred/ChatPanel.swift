import AppKit
import FredKit

/// The chat panel's header over the chat window's header rows, as Claude
/// Code's VS Code panel: Claude's mark and the conversation's title with
/// history, new chat and close; then the model and permission mode.
final class ChatHeader: NSView {
    var send: (_ op: String, _ arg: String?) -> Void = { _, _ in }
    var editorBg = NSColor.black { didSet { needsDisplay = true } }
    /// The clock: opens or closes the Sessions list.
    var onHistory: () -> Void = {}
    let history: Pill

    /// Claude's terracotta.
    static let claude = NSColor(srgbRed: 0xD9 / 255, green: 0x77 / 255, blue: 0x57 / 255, alpha: 1)

    private let mark = Pill(title: "")
    private let title = NSTextField(labelWithString: "New Chat")
    private let model = Pill(title: "")
    private let mode = Pill(title: "")
    private let status = NSTextField(labelWithString: "")

    override init(frame: NSRect) {
        history = ChatHeader.icon("clock.arrow.circlepath", "Past Conversations") {}
        super.init(frame: frame)
        mark.image = NSImage(systemSymbolName: "sparkle", accessibilityDescription: "Claude")
        mark.square = true
        mark.tint = ChatHeader.claude
        mark.weight = .semibold
        mark.onClick = { [unowned self] in self.send("new", nil) }
        mark.toolTip = "Claude"
        title.font = .systemFont(ofSize: 13, weight: .semibold)
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        status.font = .systemFont(ofSize: 11)
        status.textColor = .secondaryLabelColor
        history.onClick = { [unowned self] in self.onHistory() }
        let new = ChatHeader.icon("square.and.pencil", "New Chat") { [unowned self] in self.send("new", nil) }
        let close = ChatHeader.icon("xmark", "Close Chat") { [unowned self] in self.send("toggle", nil) }
        model.image = NSImage(systemSymbolName: "cpu", accessibilityDescription: "Model")
        mode.image = NSImage(systemSymbolName: "shield.lefthalf.filled", accessibilityDescription: "Permission mode")
        for p in [model, mode] { p.chevron = true; p.bordered = true; p.pointSize = 11 }
        model.onClick = { [unowned self] in self.send("model", nil) }
        mode.onClick = { [unowned self] in self.send("mode", nil) }
        model.toolTip = "Switch model"
        mode.toolTip = "Switch permission mode (⇧Tab in Claude Code)"
        for v in [mark, title, history, new, close, model, mode, status] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            mark.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            mark.centerYAnchor.constraint(equalTo: topAnchor, constant: 13),
            title.leadingAnchor.constraint(equalTo: mark.trailingAnchor, constant: 2),
            title.centerYAnchor.constraint(equalTo: mark.centerYAnchor),
            title.trailingAnchor.constraint(lessThanOrEqualTo: history.leadingAnchor, constant: -6),
            close.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            close.centerYAnchor.constraint(equalTo: mark.centerYAnchor),
            new.trailingAnchor.constraint(equalTo: close.leadingAnchor, constant: -2),
            new.centerYAnchor.constraint(equalTo: mark.centerYAnchor),
            history.trailingAnchor.constraint(equalTo: new.leadingAnchor, constant: -2),
            history.centerYAnchor.constraint(equalTo: mark.centerYAnchor),
            model.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            model.centerYAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            mode.leadingAnchor.constraint(equalTo: model.trailingAnchor, constant: 6),
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
        let t = c.title ?? "New Chat"
        if title.stringValue != t { title.stringValue = t; title.toolTip = t }
        title.textColor = c.title == nil ? .secondaryLabelColor : .labelColor
        model.title = c.model.map(ChatHeader.modelName) ?? "Claude"
        mode.title = ChatHeader.modeName(c.mode)
        mode.tint = c.mode == "bypassPermissions" ? .systemRed : c.mode == "plan" ? .systemTeal
            : c.mode == "acceptEdits" || c.mode == "auto" ? .systemPurple : nil
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

    private static func icon(_ symbol: String, _ tip: String, _ run: @escaping () -> Void) -> Pill {
        let b = Pill(title: "")
        b.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
        b.toolTip = tip
        b.onClick = run
        b.square = true
        return b
    }
}

/// A borderless button: text, an icon, or both; a faint fill on hover.
final class Pill: NSView {
    var title: String { didSet { if title != oldValue { invalidateIntrinsicContentSize(); needsDisplay = true } } }
    var image: NSImage?
    var square = false
    /// The icon's size and weight (the sidebar's add-folder + is bigger and bolder).
    var pointSize: CGFloat = 13
    var weight: NSFont.Weight = .regular
    /// An icon drawn bright, for a part of the layout that is showing.
    var lit = false { didSet { if lit != oldValue { needsDisplay = true } } }
    /// A lit filled symbol in the accent colour (the chat's send button).
    var accent = false
    /// The icon's colour, in place of the label colours.
    var tint: NSColor? { didSet { if tint != oldValue { needsDisplay = true } } }
    /// A hairline outline (a chip), and a ⌄ after the title (it opens a menu).
    var bordered = false
    var chevron = false { didSet { invalidateIntrinsicContentSize() } }
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

    /// Where the text starts: after the icon, when there's both.
    private var textX: CGFloat { image == nil ? 7 : 7 + pointSize + 5 }

    override var intrinsicContentSize: NSSize {
        square ? NSSize(width: pointSize + 9, height: pointSize + 9)
            : NSSize(width: (title as NSString).size(withAttributes: [.font: Pill.font]).width + textX + 7 + (chevron ? 16 : 0), height: 20)
    }

    override func draw(_ r: NSRect) {
        let shape = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: bordered ? bounds.height / 2 : 4, yRadius: bordered ? bounds.height / 2 : 4)
        if accent && image == nil {
            NSColor.controlAccentColor.withAlphaComponent(hover ? 1 : 0.85).setFill()
            shape.fill()
        } else if hover {
            NSColor.labelColor.withAlphaComponent(0.08).setFill()
            shape.fill()
        }
        if bordered {
            NSColor.labelColor.withAlphaComponent(hover ? 0.22 : 0.12).setStroke()
            shape.stroke()
        }
        let text = accent ? NSColor.white : hover ? NSColor.labelColor : NSColor.secondaryLabelColor
        if let image {
            let colors: [NSColor] = accent && lit ? [.white, .controlAccentColor] : [tint ?? (lit ? .labelColor : .secondaryLabelColor)]
            let cfg = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight).applying(.init(paletteColors: colors))
            let i = image.withSymbolConfiguration(cfg) ?? image
            let s = i.size
            let x = square ? (bounds.width - s.width) / 2 : 7 + (pointSize - s.width) / 2
            i.draw(in: NSRect(x: x, y: (bounds.height - s.height) / 2, width: s.width, height: s.height),
                   from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        }
        guard !square, !title.isEmpty else { return }
        let t = NSAttributedString(string: title, attributes: [.font: Pill.font, .foregroundColor: text])
        t.draw(at: NSPoint(x: textX, y: (bounds.height - t.size().height) / 2))
        if chevron, let c = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 8, weight: .semibold).applying(.init(paletteColors: [.secondaryLabelColor]))) {
            c.draw(in: NSRect(x: bounds.maxX - 9 - c.size.width, y: (bounds.height - c.size.height) / 2, width: c.size.width, height: c.size.height),
                   from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
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

/// A chat with nothing said yet: Claude's mark, a greeting, and things to
/// ask, as Claude Code's and Copilot's empty chat panels.
final class ChatWelcome: NSView {
    var send: (_ op: String, _ arg: String?) -> Void = { _, _ in }
    static let asks = [
        ("doc.text.magnifyingglass", "Explain the current file"),
        ("ladybug", "Find bugs in the current file"),
        ("checkmark.seal", "Write tests for the current file"),
        ("arrow.triangle.branch", "Summarize uncommitted changes"),
    ]
    private var chips: [Pill] = []
    private static let top: CGFloat = 92, chipH: CGFloat = 28, gap: CGFloat = 6

    override init(frame: NSRect) {
        super.init(frame: frame)
        for (symbol, ask) in Self.asks {
            let p = Pill(title: ask)
            p.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
            p.bordered = true
            p.onClick = { [unowned self] in self.send("ask", ask) }
            addSubview(p)
            chips.append(p)
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// The chips that fit under the greeting.
    private var fit: Int { max(0, min(chips.count, Int((bounds.height - Self.top - 12) / (Self.chipH + Self.gap)))) }

    /// The greeting and chips, centred as one block.
    private var y0: CGFloat {
        max(0, (bounds.height - Self.top - CGFloat(fit) * (Self.chipH + Self.gap)) / 2)
    }

    override func layout() {
        super.layout()
        let w = min(bounds.width - 32, 300), x = (bounds.width - w) / 2
        for (i, p) in chips.enumerated() {
            p.isHidden = i >= fit
            p.frame = NSRect(x: x, y: y0 + Self.top + CGFloat(i) * (Self.chipH + Self.gap), width: w, height: Self.chipH)
        }
        needsDisplay = true
    }

    override func draw(_ r: NSRect) {
        let y = y0
        if let s = NSImage(systemSymbolName: "sparkle", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: 30, weight: .medium).applying(.init(paletteColors: [ChatHeader.claude]))) {
            s.draw(in: NSRect(x: (bounds.width - s.size.width) / 2, y: y, width: s.size.width, height: s.size.height),
                   from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        }
        let center = NSMutableParagraphStyle()
        center.alignment = .center
        center.lineBreakMode = .byTruncatingTail
        NSAttributedString(string: "What should we build?", attributes: [
            .font: NSFont.systemFont(ofSize: 15, weight: .semibold), .foregroundColor: NSColor.labelColor, .paragraphStyle: center,
        ]).draw(in: NSRect(x: 8, y: y + 44, width: bounds.width - 16, height: 20))
        NSAttributedString(string: "Claude can read, edit and run code in this project.", attributes: [
            .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.secondaryLabelColor, .paragraphStyle: center,
        ]).draw(in: NSRect(x: 8, y: y + 66, width: bounds.width - 16, height: 16))
    }
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
        // Opaque even over the glass theme's translucent editor: open, it covers chat text.
        editorBg.withAlphaComponent(1).setFill()
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
            if let b = NSImage(systemSymbolName: "bubble.left", accessibilityDescription: nil)?
                .withSymbolConfiguration(.init(pointSize: 12, weight: .regular).applying(.init(paletteColors: [hover == i ? ChatHeader.claude : .tertiaryLabelColor]))) {
                b.draw(in: NSRect(x: f.minX + 10, y: f.minY + 7, width: b.size.width, height: b.size.height),
                       from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
            }
            let title = NSAttributedString(string: row.count > 1 ? row[1] : "", attributes: [
                .font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.labelColor, .paragraphStyle: clip])
            title.draw(in: NSRect(x: f.minX + 30, y: f.minY + 5, width: f.width - 40, height: 18))
            let ago = NSAttributedString(string: row.count > 2 ? row[2] : "", attributes: [
                .font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.tertiaryLabelColor])
            ago.draw(at: NSPoint(x: f.minX + 30, y: f.minY + 24))
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
                let b = box.insetBy(dx: 0, dy: -3)
                paintFill(ctx, b, palette.fg.withAlphaComponent(0.07), radius: 8)
                ctx.setStrokeColor(palette.fg.withAlphaComponent(0.1).cgColor)
                ctx.setLineWidth(1)
                ctx.addPath(CGPath(roundedRect: b.insetBy(dx: 0.5, dy: 0.5), cornerWidth: 8, cornerHeight: 8, transform: nil))
                ctx.strokePath()
            case "a":
                let card = box.insetBy(dx: 0, dy: -2)
                paintFill(ctx, card, NSColor.systemOrange.withAlphaComponent(0.08), radius: 5)
                paintFill(ctx, CGRect(x: card.minX, y: card.minY + 3, width: 2, height: card.height - 6), NSColor.systemOrange.withAlphaComponent(0.8), radius: 1)
            case "t":
                let card = box.insetBy(dx: 0, dy: -2)
                paintFill(ctx, card, palette.fg.withAlphaComponent(0.035), radius: 5)
                let tool = w.rows[r].map(\.s).joined().drop { !$0.isLetter }.prefix { $0.isLetter }
                paintFill(ctx, CGRect(x: card.minX, y: card.minY + 3, width: 2, height: card.height - 6), Self.toolColor(String(tool)).withAlphaComponent(0.7), radius: 1)
            case "p":
                let c = box.insetBy(dx: 0, dy: -4)
                if w.active { // a focus ring's soft glow, as macOS text fields
                    ctx.setStrokeColor(NSColor.controlAccentColor.withAlphaComponent(0.22).cgColor)
                    ctx.setLineWidth(3)
                    ctx.addPath(CGPath(roundedRect: c.insetBy(dx: -1, dy: -1), cornerWidth: 9, cornerHeight: 9, transform: nil))
                    ctx.strokePath()
                }
                paintFill(ctx, c, palette.fg.withAlphaComponent(0.04), radius: 8)
                ctx.setStrokeColor((w.active ? NSColor.controlAccentColor.withAlphaComponent(0.8) : palette.fg.withAlphaComponent(0.18)).cgColor)
                ctx.setLineWidth(1)
                ctx.addPath(CGPath(roundedRect: c.insetBy(dx: 0.5, dy: 0.5), cornerWidth: 8, cornerHeight: 8, transform: nil))
                ctx.strokePath()
                // Nothing typed yet: VS Code's placeholder after the `>`.
                if end - r == 1, w.rows[r].map(\.s).joined().trimmingCharacters(in: .whitespaces) == ">" {
                    let busy = w.chat?.busy == true
                    let hint = NSAttributedString(string: busy ? "Claude is working — C-c stops" : "Ask Claude anything — ⏎ sends, ⇧⏎ new line", attributes: [
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

    /// A tool card's bar: commands green, edits orange, reading blue, the web purple.
    static func toolColor(_ tool: String) -> NSColor {
        switch tool {
        case "Bash", "BashOutput", "KillShell": return .systemGreen
        case "Edit", "MultiEdit", "Write", "NotebookEdit": return .systemOrange
        case "Read", "Grep", "Glob", "LS": return .systemBlue
        case "WebFetch", "WebSearch": return .systemPurple
        case "Task", "Agent": return .systemPink
        default: return .controlAccentColor
        }
    }
}
