// Fred in a native window: the editor's cell grid drawn with CoreText.
//
// Look comes from ~/.config/fred/gui.conf, else ~/.config/ghostty/config,
// in Ghostty's format: font-family, font-size, theme, background,
// foreground, cursor-color, cursor-text, palette = N=#rrggbb,
// window-padding-x/y, font-thicken.

import AppKit
import CFred

// MARK: Theme

struct Theme {
    var fontFamilies: [String] = []
    var fontSize: CGFloat = 14
    var fg = NSColor(white: 0.85, alpha: 1)
    var bg = NSColor(white: 0.08, alpha: 1)
    var cursor: NSColor?
    var cursorText: NSColor?
    /// Outline glyphs a little, for heavier text (Ghostty's font-thicken).
    var thicken = true
    /// Extra line height, as Ghostty's adjust-cell-height ("20%" or points).
    var cellHeight = "20%"
    var padX: CGFloat = 8
    var padY: CGFloat = 8
    // xterm's defaults.
    var palette: [NSColor] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
        0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ].map(rgb)

    static func load() -> Theme {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let config = ["\(home)/.config/fred/gui.conf", "\(home)/.config/ghostty/config"]
            .lazy.compactMap { try? String(contentsOfFile: $0, encoding: .utf8) }.first ?? ""
        var t = Theme()
        let entries = parse(config)
        // A theme file first; the config's own settings override it.
        if let name = entries.last(where: { $0.0 == "theme" })?.1 {
            let dirs = [
                "\(home)/.config/fred/themes", "\(home)/.config/ghostty/themes",
                "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
            ]
            if let text = dirs.lazy.compactMap({
                try? String(contentsOfFile: "\($0)/\(name)", encoding: .utf8)
            }).first {
                parse(text).forEach { t.apply($0.0, $0.1) }
            }
        }
        entries.forEach { t.apply($0.0, $0.1) }
        if t.fontFamilies.isEmpty { t.fontFamilies = ["Source Code Pro"] }
        return t
    }

    static func parse(_ text: String) -> [(String, String)] {
        text.split(separator: "\n").compactMap { line in
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, !line.hasPrefix("#") else { return nil }
            let value = parts[1].trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            return (parts[0].trimmingCharacters(in: .whitespaces), value)
        }
    }

    mutating func apply(_ key: String, _ value: String) {
        switch key {
        case "font-family": fontFamilies.append(value)
        case "font-size": fontSize = CGFloat(Double(value) ?? 14)
        case "font-thicken": thicken = value != "false"
        case "adjust-cell-height": cellHeight = value
        case "foreground": fg = hex(value) ?? fg
        case "background": bg = hex(value) ?? bg
        case "cursor-color": cursor = hex(value)
        case "cursor-text": cursorText = hex(value)
        case "window-padding-x": padX = CGFloat(Double(value) ?? 8)
        case "window-padding-y": padY = CGFloat(Double(value) ?? 8)
        case "palette":
            let p = value.split(separator: "=", maxSplits: 1)
            if p.count == 2, let i = Int(p[0]), (0..<16).contains(i), let c = hex(String(p[1])) {
                palette[i] = c
            }
        default: break
        }
    }

    /// A cell color from src/gui.rs: 0 default, 0x01 palette index, 0x02 RGB.
    func color(_ c: UInt32, default d: NSColor) -> NSColor {
        switch c >> 24 {
        case 1:
            let i = Int(c & 0xff)
            if i < 16 { return palette[i] }
            if i >= 232 { return NSColor(white: CGFloat(8 + (i - 232) * 10) / 255, alpha: 1) }
            let n = i - 16, level = { (v: Int) in CGFloat(v == 0 ? 0 : 55 + v * 40) / 255 }
            return NSColor(srgbRed: level(n / 36), green: level(n / 6 % 6), blue: level(n % 6), alpha: 1)
        case 2: return rgb(Int(c & 0xffffff))
        default: return d
        }
    }
}

func rgb(_ v: Int) -> NSColor {
    NSColor(srgbRed: CGFloat(v >> 16 & 0xff) / 255, green: CGFloat(v >> 8 & 0xff) / 255,
            blue: CGFloat(v & 0xff) / 255, alpha: 1)
}

func hex(_ s: String) -> NSColor? {
    Int(s.hasPrefix("#") ? String(s.dropFirst()) : s, radix: 16).map(rgb)
}

// MARK: The editor view

final class FredView: NSView {
    let g: OpaquePointer
    var theme = Theme.load()
    var fonts: [NSFont] = []  // regular, bold, italic, bold italic
    var fallback: NSFont?
    var cell = CGSize(width: 8, height: 16)
    /// Text sits this far down its cell, centred in the extra line height.
    var textY: CGFloat = 0
    var scrolled: CGFloat = 0

    init(g: OpaquePointer) {
        self.g = g
        super.init(frame: .zero)
        setFonts(size: theme.fontSize)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    /// Below the title bar, which the background runs under.
    var top: CGFloat { theme.padY + safeAreaInsets.top }
    override var acceptsFirstResponder: Bool { true }

    func setFonts(size: CGFloat) {
        theme.fontSize = max(6, size)
        let fm = NSFontManager.shared
        let named = theme.fontFamilies.compactMap { fm.font(withFamily: $0, traits: [], weight: 5, size: theme.fontSize) }
        let base = named.first ?? .monospacedSystemFont(ofSize: theme.fontSize, weight: .regular)
        fallback = named.dropFirst().first
        let bold = fm.convert(base, toHaveTrait: .boldFontMask)
        fonts = [base, bold, fm.convert(base, toHaveTrait: .italicFontMask),
                 fm.convert(bold, toHaveTrait: .italicFontMask)]
        let advance = ("M" as NSString).size(withAttributes: [.font: base]).width
        let natural = ceil(base.ascender - base.descender + base.leading)
        let adjust = theme.cellHeight.hasSuffix("%")
            ? natural * CGFloat(Double(theme.cellHeight.dropLast()) ?? 0) / 100
            : CGFloat(Double(theme.cellHeight) ?? 0)
        cell = CGSize(width: advance, height: max(natural, (natural + adjust).rounded()))
        textY = ((cell.height - natural) / 2).rounded()
        resized()
        needsDisplay = true
    }

    func resized() {
        let cols = (bounds.width - 2 * theme.padX) / cell.width
        let rows = (bounds.height - top - theme.padY) / cell.height
        fred_resize(g, UInt16(clamping: max(1, Int(cols))), UInt16(clamping: max(1, Int(rows))))
    }

    override func setFrameSize(_ size: NSSize) {
        super.setFrameSize(size)
        resized()
    }

    override func layout() {
        super.layout()
        resized()
    }

    /// Cells x0..<x1 of row y, snapped to the screen's pixels.
    func box(_ x0: Int, _ x1: Int, _ y: Int) -> CGRect {
        let scale = window?.backingScaleFactor ?? 1
        func px(_ v: CGFloat) -> CGFloat { (v * scale).rounded() / scale }
        let left = px(theme.padX + CGFloat(x0) * cell.width), right = px(theme.padX + CGFloat(x1) * cell.width)
        let t = px(top + CGFloat(y) * cell.height)
        return CGRect(x: left, y: t, width: right - left, height: px(top + CGFloat(y + 1) * cell.height) - t)
    }

    override func draw(_ dirtyRect: NSRect) {
        let f = fred_render(g)
        theme.bg.setFill()
        bounds.fill()
        guard f.cols > 0, let cells = f.cells else { return }
        let cols = Int(f.cols)
        let text = UnsafeBufferPointer(start: f.text, count: f.text_len)
        func str(_ c: FredCell) -> String {
            String(decoding: UnsafeBufferPointer(rebasing: text[Int(c.text_start)..<Int(c.text_start + c.text_len)]), as: UTF8.self)
        }
        func colors(_ c: FredCell) -> (NSColor, NSColor) {
            let fg = theme.color(c.fg, default: theme.fg), bg = theme.color(c.bg, default: theme.bg)
            return c.flags & 8 != 0 ? (bg, fg) : (fg, bg)
        }
        /// Where cell text goes.
        func origin(_ x: Int, _ y: Int) -> CGPoint {
            CGPoint(x: theme.padX + CGFloat(x) * cell.width, y: top + CGFloat(y) * cell.height + textY)
        }
        func attrs(_ c: FredCell, _ fg: NSColor, font: NSFont? = nil) -> [NSAttributedString.Key: Any] {
            var a: [NSAttributedString.Key: Any] = [
                .font: font ?? fonts[Int(c.flags & 1) + Int(c.flags & 2)],
                .foregroundColor: c.flags & 16 != 0 ? fg.withAlphaComponent(0.6) : fg,
            ]
            if theme.thicken { a[.strokeWidth] = -2.0 }
            if c.flags & 4 != 0 { a[.underlineStyle] = NSUnderlineStyle.single.rawValue }
            if c.flags & 32 != 0 { a[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            return a
        }
        /// One cell's text alone, in the fallback font if the main one lacks it.
        func drawCell(_ c: FredCell, _ s: String, _ fg: NSColor, at p: CGPoint) {
            var font: NSFont?
            if let fb = fallback, !fonts[0].covers(s) { font = fb }
            (s as NSString).draw(at: p, withAttributes: attrs(c, fg, font: font))
        }

        for y in 0..<Int(f.rows) {
            let row = UnsafeBufferPointer(start: cells + y * cols, count: cols)
            // Backgrounds: one rect per run of a color, on whole pixels, so
            // no seams show between cells.
            var x0 = 0
            while x0 < cols {
                let bg = colors(row[x0]).1
                var x1 = x0 + 1
                while x1 < cols, colors(row[x1]).1 == bg { x1 += 1 }
                if bg != theme.bg {
                    bg.setFill()
                    box(x0, x1, y).fill()
                }
                x0 = x1
            }
            // Text: runs of plain ASCII in one style share a draw call;
            // anything else is drawn alone in its own cell, so a glyph from
            // a fallback font can't shift the rest of the line.
            var x = 0
            while x < cols {
                let c = row[x]
                let s = str(c)
                if s.isEmpty || s == " " { x += 1; continue }
                let fg = colors(c).0
                if !s.allSatisfy({ $0.isASCII }) {
                    drawCell(c, s, fg, at: origin(x, y))
                    x += 1
                    continue
                }
                var run = s
                var end = x + 1
                while end < cols, row[end].fg == c.fg, row[end].bg == c.bg, row[end].flags == c.flags {
                    let t = str(row[end])
                    guard t.count == 1, t.allSatisfy({ $0.isASCII }) else { break }
                    run += t
                    end += 1
                }
                (run as NSString).draw(at: origin(x, y), withAttributes: attrs(c, fg))
                x = end
            }
        }

        // The cursor: a block (normal mode) over the cell, or a bar (insert).
        guard f.cursor_visible, f.cursor_x < f.cols, f.cursor_y < f.rows else { return }
        let i = Int(f.cursor_y) * cols + Int(f.cursor_x)
        let p = origin(Int(f.cursor_x), Int(f.cursor_y))
        let (fg, bg) = colors(cells[i])
        (theme.cursor ?? fg).setFill()
        var r = box(Int(f.cursor_x), Int(f.cursor_x) + 1, Int(f.cursor_y))
        if f.cursor_bar {
            r.size.width = 2
            r.fill()
        } else {
            r.fill()
            let s = str(cells[i])
            if !s.isEmpty { drawCell(cells[i], s, theme.cursorText ?? bg, at: p) }
        }
    }

    // MARK: Input

    override func keyDown(with e: NSEvent) {
        NSCursor.setHiddenUntilMouseMoves(true)
        let m = e.modifierFlags
        let mods: UInt32 = (m.contains(.control) ? 1 : 0) | (m.contains(.option) ? 2 : 0)
            | (m.contains(.shift) ? 4 : 0)
        let special: [UInt16: UInt32] = [
            53: 1, 36: 2, 76: 2, 51: 3, 48: 4, 126: 6, 125: 7, 123: 8, 124: 9,
            115: 10, 119: 11, 116: 12, 121: 13, 117: 14,
        ]
        if let code = special[e.keyCode] {
            fred_key(g, code, 0, mods)
        } else {
            // Option is Alt (Meta), as Ghostty's macos-option-as-alt.
            let chars = mods & 3 != 0 ? e.charactersIgnoringModifiers : e.characters
            for u in (chars ?? "").unicodeScalars { fred_key(g, 0, u.value, mods) }
        }
        changed()
    }

    func cellAt(_ e: NSEvent) -> (UInt16, UInt16) {
        let p = convert(e.locationInWindow, from: nil)
        let col = max(0, (p.x - theme.padX) / cell.width), row = max(0, (p.y - top) / cell.height)
        return (UInt16(clamping: Int(col)), UInt16(clamping: Int(row)))
    }

    func mouse(_ kind: UInt32, _ e: NSEvent) {
        let (c, r) = cellAt(e)
        fred_mouse(g, kind, c, r)
        changed()
    }

    override func mouseDown(with e: NSEvent) {
        // The strip under the hidden title bar still moves the window.
        if convert(e.locationInWindow, from: nil).y < safeAreaInsets.top {
            if e.clickCount == 2 { window?.performZoom(nil) } else { window?.performDrag(with: e) }
            return
        }
        mouse(0, e)
    }
    override func mouseDragged(with e: NSEvent) { mouse(1, e) }
    override func mouseUp(with e: NSEvent) { mouse(2, e) }

    override func scrollWheel(with e: NSEvent) {
        // Fred scrolls three lines a wheel step; a trackpad's pixels add up to steps.
        guard e.hasPreciseScrollingDeltas else {
            if e.scrollingDeltaY != 0 { mouse(e.scrollingDeltaY > 0 ? 3 : 4, e) }
            return
        }
        scrolled += e.scrollingDeltaY
        let step = cell.height * 3
        while abs(scrolled) >= step {
            mouse(scrolled > 0 ? 3 : 4, e)
            scrolled -= scrolled > 0 ? step : -step
        }
    }

    @objc func paste(_ sender: Any?) {
        guard let s = NSPasteboard.general.string(forType: .string) else { return }
        fred_paste(g, s)
        changed()
    }

    @objc func save(_ sender: Any?) { fred_save(g); changed() }
    @objc func bigger(_ sender: Any?) { setFonts(size: theme.fontSize + 1) }
    @objc func smaller(_ sender: Any?) { setFonts(size: theme.fontSize - 1) }

    /// After input: background work, then a redraw.
    func changed() {
        _ = poll()
        needsDisplay = true
    }

    /// Fred's background work. False once Fred has quit.
    @discardableResult
    func poll() -> Bool {
        let bits = fred_tick(g)
        if bits & 4 != 0 {
            let a = NSAlert()
            a.messageText = "Fred crashed"
            a.informativeText = "Unsaved changes are in the swap file: open the file again to recover them."
            a.runModal()
            exit(1)
        }
        if bits & 2 != 0 {
            fred_free(g)
            exit(0)
        }
        if bits & 1 != 0 { needsDisplay = true }
        window?.title = String(cString: fred_title(g))
        return true
    }
}

extension NSFont {
    func covers(_ s: String) -> Bool {
        let utf16 = Array(s.utf16)
        var glyphs = [CGGlyph](repeating: 0, count: utf16.count)
        return CTFontGetGlyphsForCharacters(self, utf16, &glyphs, utf16.count)
    }
}

// MARK: App

final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var view: FredView!
    var window: NSWindow!

    func applicationDidFinishLaunching(_ n: Notification) {
        let args = CommandLine.arguments.dropFirst().filter { !$0.hasPrefix("-psn_") }
        let cargs = args.map { UnsafePointer(strdup($0)) }
        guard let g = fred_new(cargs, cargs.count) else {
            let a = NSAlert()
            a.messageText = "Fred couldn't open"
            a.informativeText = String(cString: fred_error())
            a.runModal()
            exit(1)
        }
        view = FredView(g: g)
        let t = view.theme
        let size = CGSize(width: view.cell.width * 100 + 2 * t.padX, height: view.cell.height * 36 + 2 * t.padY)
        window = NSWindow(contentRect: CGRect(origin: .zero, size: size),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                          backing: .buffered, defer: false)
        // No title bar of its own: the editor's background fills the window.
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.backgroundColor = t.bg
        let light = (t.bg.usingColorSpace(.sRGB)?.brightnessComponent ?? 0) > 0.5
        window.appearance = NSAppearance(named: light ? .aqua : .darkAqua)
        window.contentView = view
        window.delegate = self
        window.setFrameAutosaveName("FredWindow")
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(view)
        view.poll()
        let timer = Timer(timeInterval: 0.05, repeats: true) { [weak self] _ in self?.view.poll() }
        RunLoop.main.add(timer, forMode: .common)
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Closing is `:q`: refused, with Fred's message, while changes are unsaved.
    func windowShouldClose(_ w: NSWindow) -> Bool {
        fred_close(view.g)
        view.changed()
        return false
    }

    func applicationShouldTerminate(_ app: NSApplication) -> NSApplication.TerminateReply {
        _ = windowShouldClose(window)
        return .terminateCancel
    }
}

func menu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
    let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
    item.submenu = NSMenu(title: title)
    items.forEach { item.submenu!.addItem($0) }
    return item
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
let main = NSMenu()
main.addItem(menu("Fred", [NSMenuItem(title: "Quit Fred", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")]))
main.addItem(menu("File", [
    NSMenuItem(title: "Save", action: #selector(FredView.save(_:)), keyEquivalent: "s"),
    NSMenuItem(title: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w"),
]))
main.addItem(menu("Edit", [NSMenuItem(title: "Paste", action: #selector(FredView.paste(_:)), keyEquivalent: "v")]))
main.addItem(menu("View", [
    NSMenuItem(title: "Bigger", action: #selector(FredView.bigger(_:)), keyEquivalent: "+"),
    NSMenuItem(title: "Smaller", action: #selector(FredView.smaller(_:)), keyEquivalent: "-"),
]))
app.mainMenu = main
app.run()
