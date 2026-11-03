import AppKit
import FredKit

/// VS Code's breadcrumbs under a window's tabs: `src › gui › frame.rs`.
/// A folder opens in dired in that window; the file reveals in the sidebar.
final class Breadcrumbs: NSView {
    var open: (String) -> Void = { _ in }
    var reveal: (String) -> Void = { _ in }
    var editorBg = NSColor.black

    private var crumbs: [(name: String, path: String)] = []
    private var frames: [CGRect] = []
    private var hover: Int?

    private static let font = NSFont.systemFont(ofSize: 13)
    private static let sep = " › "

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(path: String?, root: String) {
        let c = path.map { FileTree.crumbs($0, root: root) } ?? []
        guard c.map(\.path) != crumbs.map(\.path) else { return }
        crumbs = c
        hover = nil
        needsDisplay = true
    }

    private func text(_ s: String, _ i: Int) -> NSAttributedString {
        NSAttributedString(string: s, attributes: [
            .font: Breadcrumbs.font,
            .foregroundColor: i == hover ? NSColor.labelColor : NSColor.secondaryLabelColor,
        ])
    }

    override func draw(_ r: NSRect) {
        editorBg.setFill()
        bounds.fill()
        frames = []
        var x: CGFloat = 12
        for (i, c) in crumbs.enumerated() {
            let last = i == crumbs.count - 1
            let start = x
            if last {
                FileIcon.image(for: c.name, isDir: false, size: 13)
                    .draw(in: CGRect(x: x, y: bounds.midY - 6.5, width: 13, height: 13),
                          from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
                x += 17
            }
            let t = text(c.name, i), z = t.size()
            t.draw(at: NSPoint(x: x, y: bounds.midY - z.height / 2))
            x += z.width
            frames.append(CGRect(x: start - 3, y: 2, width: x - start + 6, height: bounds.height - 4))
            if !last {
                let s = text(Breadcrumbs.sep, -1), sz = s.size()
                s.draw(at: NSPoint(x: x, y: bounds.midY - sz.height / 2))
                x += sz.width
            }
        }
        if let h = hover, h < frames.count {
            NSColor.white.withAlphaComponent(0.06).setFill()
            NSBezierPath(roundedRect: frames[h], xRadius: 3, yRadius: 3).fill()
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    private func index(_ e: NSEvent) -> Int? {
        let p = convert(e.locationInWindow, from: nil)
        return frames.firstIndex { $0.contains(p) }
    }

    override func mouseMoved(with e: NSEvent) {
        let h = index(e)
        if h != hover { hover = h; needsDisplay = true }
    }

    override func mouseExited(with e: NSEvent) { hover = nil; needsDisplay = true }
    override func mouseDown(with e: NSEvent) {}
    override func rightMouseDown(with e: NSEvent) {} // not a click on the core's header row

    override func mouseUp(with e: NSEvent) {
        guard let i = index(e), i < crumbs.count else { return }
        i == crumbs.count - 1 ? reveal(crumbs[i].path) : open(crumbs[i].path)
    }
}
