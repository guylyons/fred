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

/// Wheel steps for one scroll event: up positive. A notched wheel sends one
/// per notch; a trackpad's points add up in `acc`, `unit` per step, starting
/// over when the direction turns.
public func scrollLines(_ dy: CGFloat, precise: Bool, unit: CGFloat = 3, acc: inout CGFloat) -> Int {
    guard precise else { return dy > 0 ? 1 : dy < 0 ? -1 : 0 }
    if dy * acc < 0 { acc = 0 }
    acc += dy
    let n = Int(acc / unit)
    acc -= CGFloat(n) * unit
    return n
}

/// The cursor's blink, VS Code's "smooth": solid for `hold` seconds after
/// the last change, then a cosine fade out and in every `period` seconds.
public enum Blink {
    public static func alpha(_ t: Double, hold: Double = 0.6, period: Double = 1.1) -> CGFloat {
        guard t > hold else { return 1 }
        let p = (t - hold).truncatingRemainder(dividingBy: period) / period
        return CGFloat(0.5 + 0.5 * cos(2 * .pi * p))
    }
}

/// Chrome's overscroll: past an end the text follows the fingers with more
/// and more resistance up to `limit` points, then springs back.
public enum Rubber {
    /// `offset` moved by scroll delta `d`; back toward 0 freely, stopping at 0.
    public static func pull(_ offset: CGFloat, by d: CGFloat, limit: CGFloat = 140) -> CGFloat {
        if offset != 0, (offset > 0) != (d > 0) {
            let back = offset + d
            return (back > 0) == (offset > 0) ? back : 0
        }
        let resist = max(0, 1 - abs(offset) / limit)
        let next = offset + d * 0.5 * resist
        return max(-limit, min(limit, next))
    }

    /// One 60 Hz step of the spring back; 0 once it's under half a point.
    public static func settle(_ offset: CGFloat) -> CGFloat {
        let next = offset * 0.8
        return abs(next) < 0.5 ? 0 : next
    }
}
