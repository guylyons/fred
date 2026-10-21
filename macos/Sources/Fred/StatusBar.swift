import AppKit
import FredKit

/// Native status strip laid over a window's last cell row: a blur with the
/// text view on top (a view's own drawing sits under its subviews, so the
/// text can't be drawn by the view that holds the blur).
final class StatusBar: NSView {
    private let blur = NSVisualEffectView()
    private let content = StatusContent()

    override init(frame: NSRect) {
        super.init(frame: frame)
        blur.material = .titlebar
        blur.blendingMode = .withinWindow
        blur.state = .active
        for v in [blur, content] {
            v.frame = bounds
            v.autoresizingMask = [.width, .height]
            addSubview(v)
        }
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

    override var isFlipped: Bool { true }

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

        let right = NSAttributedString(string: [s.branch, s.pos].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "  "),
                                       attributes: [.font: font, .foregroundColor: NSColor.secondaryLabelColor])
        right.draw(at: NSPoint(x: bounds.maxX - right.size().width - 8, y: mid - right.size().height / 2))
    }
}
