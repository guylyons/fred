import CoreGraphics

/// Sublime's minimap geometry: 1 pt per column, 2 pt per line. When the
/// lines are taller than the view, the map scrolls in proportion to the
/// editor's `top`.
public enum MinimapMath {
    public static let lineHeight: CGFloat = 2
    public static let width: CGFloat = 96

    /// How far the map is scrolled, in pt.
    public static func offset(top: Int, lines: Int, visible: Int, height: CGFloat) -> CGFloat {
        let extra = CGFloat(lines) * lineHeight - height
        guard extra > 0 else { return 0 }
        return min(1, CGFloat(top) / CGFloat(max(1, lines - visible))) * extra
    }

    /// The buffer line at view `y`.
    public static func line(atY y: CGFloat, offset: CGFloat) -> Int {
        Int(((y + offset) / lineHeight).rounded(.down))
    }

    /// The `top` that puts `line` in the middle of the editor, clamped.
    public static func topForCentering(line: Int, visible: Int, lines: Int) -> Int {
        clampTop(line - visible / 2, visible: visible, lines: lines)
    }

    /// The viewport box's top in the view.
    public static func boxY(top: Int, lines: Int, visible: Int, height: CGFloat) -> CGFloat {
        CGFloat(top) * lineHeight - offset(top: top, lines: lines, visible: visible, height: height)
    }

    /// The inverse of `boxY`: the `top` for a box dragged to `y`, clamped.
    public static func top(forBoxY y: CGFloat, lines: Int, visible: Int, height: CGFloat) -> Int {
        let scrolls = CGFloat(lines) * lineHeight > height
        // boxY = top * (H - 2V) / (L - V) when the map scrolls; H > 2V as rows are taller than 2 pt.
        let t = scrolls ? y * CGFloat(max(1, lines - visible)) / max(1, height - CGFloat(visible) * lineHeight)
                        : y / lineHeight
        return clampTop(Int(t.rounded()), visible: visible, lines: lines)
    }

    static func clampTop(_ t: Int, visible: Int, lines: Int) -> Int {
        min(max(0, t), max(0, lines - visible))
    }
}
