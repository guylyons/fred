import CoreGraphics

/// Sublime's tab sizing: natural widths clamped to 90...220; when they don't
/// fit, the widest shrink first (to one equal cap) down to 90, then the
/// strip scrolls.
public enum TabMath {
    public static let minWidth: CGFloat = 90
    public static let maxWidth: CGFloat = 220

    /// Frames in strip content coordinates (height 0: the strip sets it) and
    /// the scroll offset that keeps `current` fully visible, staying at
    /// `offset` (the last one) when it already does.
    public static func layout(widths: [CGFloat], available: CGFloat, current: Int,
                              offset: CGFloat = 0) -> (frames: [CGRect], offset: CGFloat) {
        var w = widths.map { min(maxWidth, max(minWidth, $0)) }
        if w.reduce(0, +) > available {
            // The cap c with sum(min(w, c)) == available, never below 90.
            var left = available, cap = minWidth
            for (i, x) in w.sorted().enumerated() {
                let c = left / CGFloat(w.count - i)
                if x >= c { cap = c; break }
                left -= x
            }
            cap = max(minWidth, cap)
            w = w.map { min($0, cap) }
        }
        var x: CGFloat = 0
        let frames = w.map { width in defer { x += width }; return CGRect(x: x, y: 0, width: width, height: 0) }
        var off = offset
        if frames.indices.contains(current) {
            let f = frames[current]
            off = min(max(off, f.maxX - available), f.minX)
        }
        return (frames, min(max(0, off), max(0, x - available)))
    }
}
