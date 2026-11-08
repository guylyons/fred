import AppKit
import FredKit

/// VS Code's breadcrumbs under a window's tabs: `src › gui › frame.rs › ƒ build`.
/// A path crumb drops down its folder's entries (folders expand, a file
/// opens in that window); the symbol crumb lists the file's symbols.
final class Breadcrumbs: NSView, NSMenuDelegate {
    var open: (String) -> Void = { _ in }
    var reveal: (String) -> Void = { _ in }
    var symbols: () -> Void = {}
    var editorBg = NSColor.black

    private var crumbs: [(name: String, path: String)] = []
    private var symbol: [String]?
    private var frames: [CGRect] = []
    private var hover: Int?

    private static let font = NSFont.systemFont(ofSize: 13)
    private static let sep = " › "

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { editorBg.alphaComponent == 1 }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func update(path: String?, root: String, symbol: [String]?) {
        let c = path.map { FileTree.crumbs($0, root: root) } ?? []
        guard c.map(\.path) != crumbs.map(\.path) || symbol != self.symbol else { return }
        crumbs = c
        self.symbol = c.isEmpty ? nil : symbol
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
        let names = crumbs.map(\.name) + (symbol.map { [$0[0]] } ?? [])
        for (i, name) in names.enumerated() {
            let last = i == names.count - 1
            let start = x
            if let icon = i == crumbs.count - 1 ? FileIcon.image(for: name, isDir: false, size: 13)
                : i == crumbs.count ? Breadcrumbs.symbolIcon(symbol?.last) : nil {
                icon.draw(in: CGRect(x: x, y: bounds.midY - 6.5, width: 13, height: 13),
                          from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
                x += 17
            }
            let t = text(name, i), z = t.size()
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
        guard let i = index(e) else { return }
        guard i < crumbs.count else { return symbols() }
        let menu = NSMenu()
        menu.delegate = self
        fill(menu, dir: (crumbs[i].path as NSString).deletingLastPathComponent, current: crumbs[i].path)
        menu.popUp(positioning: nil, at: NSPoint(x: frames[i].minX, y: bounds.maxY), in: self)
    }

    /// `kind`'s icon, VS Code's: ƒ for functions, a cube for types.
    private static func symbolIcon(_ kind: String?) -> NSImage? {
        let name = switch kind {
        case "function", "macro": "function"
        case "heading": "number"
        case "module": "shippingbox"
        case "constant", "variable": "character.textbox"
        default: "cube"
        }
        let i = NSImage(systemSymbolName: name, accessibilityDescription: kind)
        return i?.withSymbolConfiguration(.init(paletteColors: [.systemPurple]))
    }

    /// `dir`'s entries, folders first; `current` checked. A folder's entries load when it opens.
    private func fill(_ menu: NSMenu, dir: String, current: String?) {
        let fm = FileManager.default
        let names = (try? fm.contentsOfDirectory(atPath: dir)) ?? []
        let entries = names.map { n -> (String, String, Bool) in
            let p = (dir as NSString).appendingPathComponent(n)
            var isDir: ObjCBool = false
            fm.fileExists(atPath: p, isDirectory: &isDir)
            return (n, p, isDir.boolValue)
        }.sorted { ($0.2 ? 0 : 1, $0.0.lowercased()) < ($1.2 ? 0 : 1, $1.0.lowercased()) }
        for (n, p, isDir) in entries {
            let item = NSMenuItem(title: n, action: isDir ? nil : #selector(pick(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = p
            item.image = FileIcon.image(for: n, isDir: isDir, size: 14)
            item.state = p == current ? .on : .off
            if isDir {
                let sub = NSMenu(title: p)
                sub.delegate = self
                item.submenu = sub
            }
            menu.addItem(item)
        }
        if entries.isEmpty { menu.addItem(withTitle: "Empty", action: nil, keyEquivalent: "").isEnabled = false }
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        guard menu.items.isEmpty, menu.supermenu != nil else { return }
        fill(menu, dir: menu.title, current: nil)
    }

    @objc private func pick(_ item: NSMenuItem) {
        if let p = item.representedObject as? String { open(p) }
    }
}
