import AppKit

/// Rounds `v` to the nearest device pixel.
public func snap(_ v: CGFloat, scale: CGFloat) -> CGFloat {
    (v * scale).rounded() / scale
}

/// One character cell: the advance of "M" wide, the line height times 1.25
/// rounded up to whole device pixels.
public func cellSize(font: NSFont, scale: CGFloat) -> CGSize {
    var ch: UniChar = 77 // "M"
    var g: CGGlyph = 0
    CTFontGetGlyphsForCharacters(font, &ch, &g, 1)
    let w = CTFontGetAdvancesForGlyphs(font, .horizontal, &g, nil, 1)
    let h = (font.ascender - font.descender + font.leading) * 1.25
    return CGSize(width: w, height: ceil(h * scale) / scale)
}

/// Columns `s` takes; wide (CJK, emoji) characters take 2.
public func cellWidth(_ s: String) -> Int {
    // ponytail: hand-picked East Asian Wide ranges, a full EAW table if the core and this disagree
    s.unicodeScalars.reduce(0) { n, u in
        let v = u.value
        if u.properties.generalCategory == .nonspacingMark || v == 0x200d || (0xfe00...0xfe0f).contains(v) { return n }
        let wide = (0x1100...0x115f).contains(v) || (0x2e80...0xa4cf).contains(v) || (0xac00...0xd7a3).contains(v)
            || (0xf900...0xfaff).contains(v) || (0xfe30...0xfe4f).contains(v) || (0xff00...0xff60).contains(v)
            || (0xffe0...0xffe6).contains(v) || (0x1f300...0x1f64f).contains(v) || (0x1f900...0x1f9ff).contains(v)
            || (0x20000...0x3fffd).contains(v)
        return n + (wide ? 2 : 1)
    }
}
