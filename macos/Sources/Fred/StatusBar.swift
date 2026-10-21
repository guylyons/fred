import AppKit
import FredKit

/// Native status strip laid over a window's last cell row.
final class StatusBar: NSView {
    private let blur = NSVisualEffectView()
    private var status: Status?

    override init(frame: NSRect) {
        super.init(frame: frame)
        blur.material = .titlebar
        blur.blendingMode = .withinWindow
        blur.state = .active
        blur.autoresizingMask = [.width, .height]
        blur.frame = bounds
        addSubview(blur, positioned: .below, relativeTo: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func update(_ s: Status, active: Bool) {
        blur.alphaValue = active ? 1 : 0.5
        guard s != status else { return }
        status = s
        needsDisplay = true
    }

    static func color(_ mode: String) -> NSColor {
        switch mode {
        case "NORMAL": .systemBlue
        case "INSERT": .systemGreen
        case "VISUAL": .systemOrange
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
        StatusBar.color(s.mode).setFill()
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
