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
        if hover {
            NSColor.white.withAlphaComponent(0.08).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()
        }
        if let image {
            let cfg = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
                .applying(.init(paletteColors: [lit ? .labelColor : .secondaryLabelColor]))
            let i = image.withSymbolConfiguration(cfg) ?? image
            let s = i.size
            i.draw(in: NSRect(x: (bounds.width - s.width) / 2, y: (bounds.height - s.height) / 2, width: s.width, height: s.height),
                   from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        } else {
            let t = NSAttributedString(string: title, attributes: [.font: Pill.font, .foregroundColor: NSColor.secondaryLabelColor])
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

    private var rows: [[String]] = []
    private var hover: Int?
    private static let headH: CGFloat = 28, rowH: CGFloat = 44, moreH: CGFloat = 28

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(_ sessions: [[String]]) {
        guard sessions != rows else { return }
        rows = sessions
        hover = nil
        needsDisplay = true
    }

    /// Rows that fit; the rest are behind More.
    private var shown: Int { max(0, min(rows.count, Int((bounds.height - Self.headH - Self.moreH) / Self.rowH))) }

    private func rowRect(_ i: Int) -> NSRect {
        NSRect(x: 6, y: Self.headH + CGFloat(i) * Self.rowH, width: bounds.width - 12, height: Self.rowH)
    }

    private var moreRect: NSRect {
        NSRect(x: 6, y: Self.headH + CGFloat(shown) * Self.rowH, width: bounds.width - 12, height: Self.moreH)
    }

    override func draw(_ r: NSRect) {
        editorBg.setFill()
        bounds.fill()
        let head = NSAttributedString(string: "Sessions", attributes: [
            .font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.secondaryLabelColor])
        head.draw(at: NSPoint(x: 12, y: 7))
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

    /// The row under `e`: its index, -1 for More, nil for none.
    private func index(_ e: NSEvent) -> Int? {
        let p = convert(e.locationInWindow, from: nil)
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
        if i == -1 { send("more", nil) } else if i < rows.count { send("resume", rows[i].first) }
    }
}
