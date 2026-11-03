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

    /// VS Code's drop zones over an editor body: within a third of an edge
    /// (the nearest one) splits that way, showing that half; the middle
    /// is `center`, the whole body. Flipped coordinates (top is minY).
    public static func dropZone(_ p: CGPoint, in body: CGRect) -> (zone: String, hint: CGRect) {
        let rx = (p.x - body.minX) / max(1, body.width), ry = (p.y - body.minY) / max(1, body.height)
        let edges: [(String, CGFloat)] = [("left", rx), ("right", 1 - rx), ("top", ry), ("bottom", 1 - ry)]
        let near = edges.min { $0.1 < $1.1 }!
        guard near.1 < 1.0 / 3 else { return ("center", body) }
        let (w, h) = (body.width / 2, body.height / 2)
        switch near.0 {
        case "left": return ("left", CGRect(x: body.minX, y: body.minY, width: w, height: body.height))
        case "right": return ("right", CGRect(x: body.minX + w, y: body.minY, width: w, height: body.height))
        case "top": return ("top", CGRect(x: body.minX, y: body.minY, width: body.width, height: h))
        default: return ("bottom", CGRect(x: body.minX, y: body.minY + h, width: body.width, height: h))
        }
    }

    /// The slot a tab dropped at strip x lands in: before the first tab
    /// whose middle is right of it. `frames` in content coordinates.
    public static func slot(atX x: CGFloat, frames: [CGRect], offset: CGFloat) -> Int {
        frames.firstIndex { $0.midX - offset > x } ?? frames.count
    }
}
