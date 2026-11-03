import AppKit
import FredKit

/// Native status strip laid over a window's last cell row: a shade darker
/// than the editor, with a hairline on top, so it reads apart from the text.
final class StatusBar: NSView {
    private let content = StatusContent()
    var editorBg: NSColor {
        get { content.bg }
        set { content.bg = newValue }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        content.frame = bounds
        content.autoresizingMask = [.width, .height]
        addSubview(content)
    }

    required init?(coder: NSCoder) { fatalError() }

    func update(_ s: Status, active: Bool) {
        alphaValue = active ? 1 : 0.5
        guard s != content.status else { return }
        content.status = s
        content.needsDisplay = true
    }
}

private final class StatusContent: NSView {
    var status: Status?
    var bg = NSColor.black

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }

    static func color(_ mode: String) -> NSColor {
        switch mode {
        case "NORMAL": .systemBlue
        case "INSERT": .systemGreen
        case "VISUAL", "SELECT", "SELECT LINE": .systemOrange
        case "TERMINAL": .systemTeal
        default: .systemPurple // COMMAND and the rest
        }
    }

    override func draw(_ dirty: NSRect) {
        bg.blended(withFraction: 0.18, of: .black)!.setFill()
        bounds.fill()
        NSColor.black.withAlphaComponent(0.45).setFill()
        NSRect(x: 0, y: 0, width: bounds.width, height: 1).fill()
        guard let s = status else { return }
        let mid = bounds.midY
        let badgeFont = NSFont.systemFont(ofSize: 11, weight: .semibold)
        let badge = NSAttributedString(string: s.mode, attributes: [.font: badgeFont, .foregroundColor: NSColor.white])
        let bs = badge.size()
        let rect = NSRect(x: 6, y: mid - (bs.height + 2) / 2, width: bs.width + 12, height: bs.height + 2)
        StatusContent.color(s.mode).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 4, yRadius: 4).fill()
        badge.draw(at: NSPoint(x: rect.minX + 6, y: rect.minY + 1))

        let font = NSFont.systemFont(ofSize: 12)
        let name = NSAttributedString(string: s.name + (s.modified ? " ●" : ""),
                                      attributes: [.font: font, .foregroundColor: NSColor.labelColor])
        name.draw(at: NSPoint(x: rect.maxX + 8, y: mid - name.size().height / 2))

        // VS Code's right side, right to left; what doesn't fit beside the name is left out.
        let parts = s.pos.split(separator: ":")
        let pos = parts.count == 2 ? "Ln \(parts[0]), Col \(parts[1])" : s.pos
        let items: [(String?, String)] = [
            (s.branch.map { _ in "arrow.triangle.branch" }, s.branch ?? ""),
            (s.lang?.isEmpty == false ? "curlybraces" : nil, s.lang ?? ""),
            (nil, s.eol ?? ""),
            (nil, s.enc ?? ""),
            (nil, s.indent ?? ""),
            (nil, pos),
        ].filter { !$0.1.isEmpty }
        let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: NSColor.secondaryLabelColor]
        var x = bounds.maxX - 10
        let floor = rect.maxX + 8 + name.size().width + 16
        for (symbol, text) in items {
            let t = NSAttributedString(string: text, attributes: attrs)
            let img = symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil) }?
                .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 10, weight: .medium)
                    .applying(.init(paletteColors: [.secondaryLabelColor])))
            let iw = img.map { $0.size.width + 4 } ?? 0
            let w = t.size().width + iw
            guard x - w >= floor else { break }
            x -= w
            if let img {
                let z = img.size
                let r = NSRect(x: x, y: mid - z.height / 2, width: z.width, height: z.height)
                img.draw(in: r, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
            }
            t.draw(at: NSPoint(x: x + iw, y: mid - t.size().height / 2))
            x -= 16
        }
    }
}
