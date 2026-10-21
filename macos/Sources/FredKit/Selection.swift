import CoreGraphics

/// The selection's outlines, Sublime style: rows that are next to each other
/// and overlap join into one shape. Each outline is its corner points,
/// clockwise in a flipped (y down) view, `cell` points per cell and x on
/// device pixels.
public func selectionOutlines(_ sel: [Sel], cell: CGSize, scale: CGFloat) -> [[CGPoint]] {
    let rows = sel.filter { $0.w > 0 }.sorted { $0.y < $1.y }
    var groups: [[Sel]] = []
    for s in rows {
        if let p = groups.last?.last, p.y + 1 == s.y, s.x < p.x + p.w, p.x < s.x + s.w {
            groups[groups.count - 1].append(s)
        } else {
            groups.append([s])
        }
    }
    func x(_ col: Int) -> CGFloat { snap(CGFloat(col) * cell.width, scale: scale) }
    func y(_ row: Int) -> CGFloat { CGFloat(row) * cell.height }
    return groups.map { g in
        // Down the right side, then back up the left.
        var pts = [CGPoint(x: x(g[0].x), y: y(g[0].y))]
        for s in g {
            pts.append(CGPoint(x: x(s.x + s.w), y: y(s.y)))
            pts.append(CGPoint(x: x(s.x + s.w), y: y(s.y + 1)))
        }
        for s in g.reversed() {
            pts.append(CGPoint(x: x(s.x), y: y(s.y + 1)))
            pts.append(CGPoint(x: x(s.x), y: y(s.y)))
        }
        return corners(pts)
    }
}

/// `pts` without repeated points and without points on a straight line.
func corners(_ pts: [CGPoint]) -> [CGPoint] {
    var out: [CGPoint] = []
    for p in pts where out.last != p { out.append(p) }
    while out.count > 1, out.first == out.last { out.removeLast() }
    var changed = true
    while changed && out.count > 2 {
        changed = false
        for i in 0..<out.count {
            let a = out[(i + out.count - 1) % out.count], b = out[i], c = out[(i + 1) % out.count]
            if (a.x == b.x && b.x == c.x) || (a.y == b.y && b.y == c.y) {
                out.remove(at: i)
                changed = true
                break
            }
        }
    }
    return out
}

/// An outline with its corners rounded by `radius` (less where an edge is
/// short): outer corners curve in, inside corners curve out.
public func roundedPath(_ outline: [CGPoint], radius: CGFloat) -> CGPath {
    let path = CGMutablePath()
    let n = outline.count
    guard n > 2 else { return path }
    func dist(_ a: CGPoint, _ b: CGPoint) -> CGFloat { hypot(a.x - b.x, a.y - b.y) }
    let last = outline[n - 1], first = outline[0]
    path.move(to: CGPoint(x: (last.x + first.x) / 2, y: (last.y + first.y) / 2))
    for i in 0..<n {
        let a = outline[(i + n - 1) % n], b = outline[i], c = outline[(i + 1) % n]
        let r = min(radius, dist(a, b) / 2, dist(b, c) / 2)
        path.addArc(tangent1End: b, tangent2End: c, radius: r)
    }
    path.closeSubpath()
    return path
}

/// The selection as one path to fill and stroke.
public func selectionPath(_ sel: [Sel], cell: CGSize, scale: CGFloat) -> CGPath {
    let path = CGMutablePath()
    for o in selectionOutlines(sel, cell: cell, scale: scale) {
        path.addPath(roundedPath(o, radius: cell.height / 4))
    }
    return path
}
