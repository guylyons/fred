import AppKit
import FredKit

/// VS Code's quick-open palette: a floating card at the top of the editor
/// for Fred's pickers (find file, grep, buffers, help…). Keys still go to
/// the core; a click picks a row.
final class PaletteView: NSView {
    var pick: (Int) -> Void = { _ in }
    var editorBg = NSColor.black {
        didSet { layer?.backgroundColor = editorBg.blended(withFraction: 0.08, of: .white)!.cgColor }
    }

    private var p: PickerFrame?
    private var hover: Int?

    static let queryH: CGFloat = 38, rowH: CGFloat = 24, pad: CGFloat = 6
    private static let ui = NSFont.systemFont(ofSize: 13)
    private static let bold = NSFont.systemFont(ofSize: 13, weight: .semibold)
    private static let mono = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let small = NSFont.systemFont(ofSize: 11)

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 8
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.white.withAlphaComponent(0.08).cgColor
        shadow = NSShadow()
        layer?.shadowColor = NSColor.black.cgColor
        layer?.shadowOpacity = 0.5
        layer?.shadowRadius = 16
        layer?.shadowOffset = CGSize(width: 0, height: -6)
        layer?.masksToBounds = false
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// Height for `p`: the query, then its rows (one line for none).
    static func height(_ p: PickerFrame) -> CGFloat {
        queryH + 1 + CGFloat(max(p.rows.count, 1)) * rowH + 2 * pad
    }

    func update(_ p: PickerFrame) {
        guard p != self.p else { return }
        self.p = p
        needsDisplay = true
    }

    private func rowRect(_ i: Int) -> CGRect {
        CGRect(x: PaletteView.pad, y: PaletteView.queryH + 1 + PaletteView.pad + CGFloat(i) * PaletteView.rowH,
               width: bounds.width - 2 * PaletteView.pad, height: PaletteView.rowH)
    }

    override func draw(_ dirty: NSRect) {
        guard let p else { return }
        // The query field: the title as a placeholder, a caret, the count on the right.
        let field = CGRect(x: 8, y: 6, width: bounds.width - 16, height: PaletteView.queryH - 12)
        NSColor.black.withAlphaComponent(0.25).setFill()
        NSBezierPath(roundedRect: field, xRadius: 4, yRadius: 4).fill()
        NSColor.controlAccentColor.withAlphaComponent(0.7).setStroke()
        let ring = NSBezierPath(roundedRect: field.insetBy(dx: 0.5, dy: 0.5), xRadius: 4, yRadius: 4)
        ring.lineWidth = 1
        ring.stroke()
        let status = NSAttributedString(string: p.status, attributes: [.font: PaletteView.small, .foregroundColor: NSColor.tertiaryLabelColor])
        let sz = status.size()
        status.draw(at: NSPoint(x: field.maxX - sz.width - 8, y: field.midY - sz.height / 2))
        let q = p.query.isEmpty
            ? NSAttributedString(string: p.title.trimmingCharacters(in: .whitespaces),
                                 attributes: [.font: PaletteView.ui, .foregroundColor: NSColor.tertiaryLabelColor])
            : NSAttributedString(string: p.query, attributes: [.font: PaletteView.ui, .foregroundColor: NSColor.labelColor])
        let qy = field.midY - q.size().height / 2
        if p.all == true, !p.query.isEmpty {
            NSColor.selectedTextBackgroundColor.setFill()
            NSRect(x: field.minX + 7, y: qy, width: q.size().width + 2, height: q.size().height).fill()
        }
        q.draw(at: NSPoint(x: field.minX + 8, y: qy))
        let before = NSAttributedString(string: String(p.query.prefix(p.caret)), attributes: [.font: PaletteView.ui])
        NSColor.controlAccentColor.setFill()
        CGRect(x: field.minX + 8 + before.size().width, y: field.midY - 8, width: 1.5, height: 16).fill()

        NSColor.white.withAlphaComponent(0.06).setFill()
        CGRect(x: 0, y: PaletteView.queryH, width: bounds.width, height: 1).fill()

        if p.rows.isEmpty {
            let none = NSAttributedString(string: "No matching results", attributes: [.font: PaletteView.ui, .foregroundColor: NSColor.secondaryLabelColor])
            none.draw(at: NSPoint(x: rowRect(0).minX + 10, y: rowRect(0).midY - none.size().height / 2))
            return
        }
        for (i, r) in p.rows.enumerated() {
            let rr = rowRect(i)
            if i == p.sel {
                NSColor.controlAccentColor.withAlphaComponent(0.35).setFill()
                NSBezierPath(roundedRect: rr, xRadius: 4, yRadius: 4).fill()
            } else if i == hover {
                NSColor.white.withAlphaComponent(0.05).setFill()
                NSBezierPath(roundedRect: rr, xRadius: 4, yRadius: 4).fill()
            }
            drawRow(r, in: rr, files: p.files)
        }
    }

    /// A file row is its name (matches bold) then its folder, dimmer; others are code-ish text.
    private func drawRow(_ r: PickerRow, in rr: CGRect, files: Bool) {
        var x = rr.minX + 10
        let text = r.text
        let isDir = text.hasSuffix("/")
        let trimmed = isDir ? String(text.dropLast()) : text
        let cut = files ? (trimmed.lastIndex(of: "/").map { trimmed.index(after: $0) } ?? trimmed.startIndex) : text.startIndex
        let nameStart = text.distance(from: text.startIndex, to: cut)
        let name = String(text[cut...])
        if files {
            let icon = FileIcon.image(for: name, isDir: isDir, size: 14)
            icon.draw(in: CGRect(x: x, y: rr.midY - 7, width: 14, height: 14), from: .zero, operation: .sourceOver,
                      fraction: 1, respectFlipped: true, hints: nil)
            x += 22
        }
        let font = files ? PaletteView.ui : PaletteView.mono
        let s = NSMutableAttributedString(string: name, attributes: [.font: font, .foregroundColor: NSColor.labelColor])
        for h in r.hl where h.count == 2 {
            let a = max(0, h[0] - nameStart), b = min(name.count, h[1] - nameStart)
            guard a < b else { continue }
            let lo = name.index(name.startIndex, offsetBy: a), hi = name.index(name.startIndex, offsetBy: b)
            s.addAttributes([.font: files ? PaletteView.bold : NSFont.monospacedSystemFont(ofSize: 12, weight: .bold),
                             .foregroundColor: NSColor.controlAccentColor.blended(withFraction: 0.4, of: .white)!],
                            range: NSRange(lo..<hi, in: name))
        }
        let para = NSMutableParagraphStyle()
        para.lineBreakMode = .byTruncatingTail
        s.addAttribute(.paragraphStyle, value: para, range: NSRange(location: 0, length: s.length))
        let h = s.size().height
        let nameW = min(s.size().width, rr.maxX - 10 - x)
        s.draw(with: CGRect(x: x, y: rr.midY - h / 2, width: nameW, height: h), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        x += nameW + 8
        if files, nameStart > 0, x < rr.maxX - 20 {
            let dir = NSAttributedString(string: String(text[..<cut].dropLast()), attributes: [
                .font: PaletteView.small, .foregroundColor: NSColor.secondaryLabelColor, .paragraphStyle: para,
            ])
            let dh = dir.size().height
            dir.draw(with: CGRect(x: x, y: rr.midY - dh / 2, width: rr.maxX - 10 - x, height: dh),
                     options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    private func row(_ e: NSEvent) -> Int? {
        guard let p else { return nil }
        let pt = convert(e.locationInWindow, from: nil)
        return p.rows.indices.first { rowRect($0).contains(pt) }
    }

    override func mouseMoved(with e: NSEvent) {
        let h = row(e)
        if h != hover { hover = h; needsDisplay = true }
    }

    override func mouseExited(with e: NSEvent) { hover = nil; needsDisplay = true }
    override func mouseDown(with e: NSEvent) {}
    override func mouseUp(with e: NSEvent) {
        if let i = row(e), let p { pick(p.first + i) }
    }
    override func scrollWheel(with e: NSEvent) {} // not the editor's
}
