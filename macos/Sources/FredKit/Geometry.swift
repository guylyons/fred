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
