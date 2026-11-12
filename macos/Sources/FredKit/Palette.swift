import AppKit

public struct Palette: Sendable {
    public var fg: NSColor
    public var bg: NSColor
    /// The selection: a fill a little lighter than `bg`, and its border.
    public var selFill = rgb(0x1f2233)
    public var selBorder = rgb(0x2d3450)

    public init(fg: NSColor, bg: NSColor) {
        self.fg = fg
        self.bg = bg
    }

    /// The default: Material Deep Ocean, matching the core's `material-deep-ocean` syntax theme.
    public static var deepOcean: Palette { Palette(fg: rgb(0xa6accd), bg: rgb(0x0f111a)) }

    /// Glass: Deep Ocean over the user's Ghostty background, black at 90%, so
    /// the blurred desktop shows through as it does behind their Neovim.
    public static var glass: Palette { Palette(fg: rgb(0xa6accd), bg: rgb(0x000000).withAlphaComponent(0.9)) }

    /// True when the background lets the desktop show through.
    public var clear: Bool { bg.alphaComponent < 1 }

    // Material ANSI set (dark gray is Deep Ocean's comment color), ratatui order: black, red, green, yellow, blue,
    // magenta, cyan, gray, then the dark gray and bright variants.
    static let ansi: [Int] = [
        0x000000, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xeeffff,
        0x464b5d, 0xff5370, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xffffff,
    ]

    static func rgb(_ v: Int) -> NSColor {
        NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255,
                blue: CGFloat(v & 255) / 255, alpha: 1)
    }

    public func color(_ code: Int?, default d: NSColor) -> NSColor {
        guard let c = code else { return d }
        if c >= 0x1000000 { return Palette.rgb(c & 0xffffff) }
        if c < 0 { return d }
        if c < 16 { return Palette.rgb(Palette.ansi[c]) }
        if c < 232 {
            let i = c - 16
            func lvl(_ n: Int) -> Int { n == 0 ? 0 : 55 + 40 * n }
            return Palette.rgb(lvl(i / 36) << 16 | lvl(i / 6 % 6) << 8 | lvl(i % 6))
        }
        if c < 256 {
            let g = 8 + 10 * (c - 232)
            return Palette.rgb(g << 16 | g << 8 | g)
        }
        return d
    }
}
