import AppKit

/// VS Code's activity bar: a column of icon buttons at the window's left
/// edge. Each runs a closure; `on` buttons (sidebar, minimap) draw lit.
final class ActivityBar: NSView {
    struct Item {
        let symbol: String
        let tip: String
        let run: () -> Void
        /// nil for a one-shot action; else whether it's currently on.
        var on: (() -> Bool)? = nil
    }

    private var buttons: [Button] = []
    /// Glass: the bar paints its own chrome tint (nothing solid behind it).
    var tint = NSColor.clear { didSet { needsDisplay = true } }

    init(top: [Item], bottom: [Item]) {
        super.init(frame: .zero)
        let stack = { (items: [Item]) -> NSStackView in
            let s = NSStackView(views: items.map { i in Button(i) })
            s.orientation = .vertical
            s.spacing = 4
            s.translatesAutoresizingMaskIntoConstraints = false
            self.addSubview(s)
            self.buttons += s.views.compactMap { $0 as? Button }
            return s
        }
        let t = stack(top), b = stack(bottom)
        NSLayoutConstraint.activate([
            t.topAnchor.constraint(equalTo: safeAreaLayoutGuide.topAnchor, constant: 6),
            t.centerXAnchor.constraint(equalTo: centerXAnchor),
            b.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            b.centerXAnchor.constraint(equalTo: centerXAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Redraw the lit states (after a toggle from the menu or a key).
    func refresh() { buttons.forEach { $0.needsDisplay = true } }

    override func draw(_ r: NSRect) {
        tint.setFill()
        bounds.fill()
        NSColor.black.withAlphaComponent(0.15).setFill()
        bounds.fill()
        NSColor.black.withAlphaComponent(0.35).setFill()
        NSRect(x: bounds.maxX - 1, y: 0, width: 1, height: bounds.height).fill()
    }

    private final class Button: NSView {
        let item: Item
        private var hover = false
        private let image: NSImage?

        init(_ item: Item) {
            self.item = item
            image = NSImage(systemSymbolName: item.symbol, accessibilityDescription: item.tip)?
                .withSymbolConfiguration(.init(pointSize: 18, weight: .regular))
            super.init(frame: .zero)
            toolTip = item.tip
            setAccessibilityElement(true)
            setAccessibilityRole(.button)
            setAccessibilityLabel(item.tip)
            translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: 36), heightAnchor.constraint(equalToConstant: 36)])
        }

        required init?(coder: NSCoder) { fatalError() }

        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
        }

        override func mouseEntered(with e: NSEvent) { hover = true; needsDisplay = true }
        override func mouseExited(with e: NSEvent) { hover = false; needsDisplay = true }
        override func acceptsFirstMouse(for e: NSEvent?) -> Bool { true }
        override func mouseDown(with e: NSEvent) {}
        override func mouseUp(with e: NSEvent) {
            guard bounds.contains(convert(e.locationInWindow, from: nil)) else { return }
            item.run()
            superview?.superview?.needsDisplay = true
            (superview?.superview as? ActivityBar)?.refresh()
        }
        override func accessibilityPerformPress() -> Bool { item.run(); return true }

        override func draw(_ r: NSRect) {
            let lit = item.on?() ?? false
            if hover || lit {
                NSColor.white.withAlphaComponent(lit ? 0.1 : 0.06).setFill()
                NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
            }
            guard let image else { return }
            let z = image.size
            let at = NSRect(x: bounds.midX - z.width / 2, y: bounds.midY - z.height / 2, width: z.width, height: z.height)
            image.draw(in: at)
            (lit || hover ? NSColor.labelColor : NSColor.secondaryLabelColor).set()
            at.fill(using: .sourceAtop)
        }
    }
}
