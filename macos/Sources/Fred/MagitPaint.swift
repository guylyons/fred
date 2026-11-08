import AppKit
import FredKit

/// Decorations use the core's cell coordinates, never a second text layout.
extension EditorView {
    func paintRect(_ x: Int, _ y: Int, _ width: Int) -> CGRect {
        CGRect(x: CGFloat(x) * cell.width, y: CGFloat(y) * cell.height,
               width: CGFloat(max(0, width)) * cell.width, height: cell.height)
    }

    func paintFill(_ ctx: CGContext, _ rect: CGRect, _ color: NSColor, radius: CGFloat = 0) {
        ctx.setFillColor(color.cgColor)
        ctx.addPath(CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
        ctx.fillPath()
    }

    func drawPaint(_ ctx: CGContext, _ w: Win) {
        guard let paint = w.paint else { return }
        let y = w.y + (w.header ?? 0)
        let width = w.w - (w.margin ?? 0) - (w.divider ? 1 : 0)
        let pointer = window.map { convert($0.mouseLocationOutsideOfEventStream, from: nil) }
        let hunk: (Int) -> Bool = { i in
            guard paint.indices.contains(i) else { return false }
            return ["hunk_header", "added", "removed", "context"].contains(paint[i].role?.kind ?? "")
        }
        for (r, p) in paint.enumerated() {
            let rect = paintRect(w.x, y + r, width)
            switch p.role?.kind {
            case "heading":
                paintFill(ctx, rect, palette.fg.withAlphaComponent(0.035))
                paintFill(ctx, CGRect(x: rect.minX, y: rect.minY, width: rect.width, height: 0.5), palette.fg.withAlphaComponent(0.06))
            case "hunk_header": paintFill(ctx, rect, NSColor.systemBlue.withAlphaComponent(0.08))
            case "added": paintFill(ctx, rect, NSColor.systemGreen.withAlphaComponent(0.13))
            case "removed": paintFill(ctx, rect, NSColor.systemRed.withAlphaComponent(0.13))
            default: break
            }
            if ["heading", "file"].contains(p.role?.kind ?? "") {
                if let pointer, rect.contains(pointer) { paintFill(ctx, rect, palette.fg.withAlphaComponent(0.06)) }
                if w.paint_gutter == 1 {
                    let open = p.role?.open ?? !w.rows[r].map(\.s).joined().trimmingCharacters(in: .whitespaces).hasSuffix("…")
                    let x = rect.minX + cell.width / 2, cy = rect.midY
                    ctx.setStrokeColor(palette.fg.withAlphaComponent(0.55).cgColor)
                    ctx.setLineWidth(1)
                    ctx.beginPath()
                    if open {
                        ctx.move(to: CGPoint(x: x - 2.5, y: cy - 1.5)); ctx.addLine(to: CGPoint(x: x, y: cy + 1.5)); ctx.addLine(to: CGPoint(x: x + 2.5, y: cy - 1.5))
                    } else {
                        ctx.move(to: CGPoint(x: x - 1.5, y: cy - 2.5)); ctx.addLine(to: CGPoint(x: x + 1.5, y: cy)); ctx.addLine(to: CGPoint(x: x - 1.5, y: cy + 2.5))
                    }
                    ctx.strokePath()
                }
            }
            if hunk(r) && (!hunk(r - 1) || p.role?.kind == "hunk_header") {
                var end = r + 1
                while hunk(end) && paint[end].role?.kind != "hunk_header" { end += 1 }
                var box = rect.insetBy(dx: 1, dy: 0)
                box.size.height = CGFloat(end - r) * cell.height
                ctx.setStrokeColor(palette.fg.withAlphaComponent(0.07).cgColor)
                ctx.setLineWidth(0.5)
                ctx.addPath(CGPath(roundedRect: box, cornerWidth: 6, cornerHeight: 6, transform: nil)); ctx.strokePath()
            }
            drawMarks(ctx, p, runs: w.rows[r], col: w.x, row: y + r)
        }
    }

    func drawMarks(_ ctx: CGContext, _ paint: RowPaint, runs: [Run], col: Int, row: Int) {
        for m in paint.marks ?? [] where m.end > m.start {
            var color: NSColor
            var radius: CGFloat = 4
            switch m.kind {
            case "count": color = palette.fg.withAlphaComponent(0.08); radius = 8
            case "ref_local": color = NSColor.systemBlue.withAlphaComponent(0.18); radius = 9
            case "ref_remote": color = palette.fg.withAlphaComponent(0.07); radius = 9
            case "key": color = NSColor.systemBlue.withAlphaComponent(0.16)
            case "status":
                let text = runs.map(\.s).joined().trimmingCharacters(in: .whitespaces)
                let tint: NSColor = text.hasPrefix("deleted") ? .systemRed : text.hasPrefix("new file") || text.hasPrefix("untracked") ? .systemGreen : text.hasPrefix("renamed") ? .systemBlue : .systemYellow
                color = tint.withAlphaComponent(0.18)
            default: continue
            }
            paintFill(ctx, paintRect(col + m.start, row, m.end - m.start).insetBy(dx: 0, dy: 1), color, radius: radius)
        }
    }

    func drawPanelPaint(_ ctx: CGContext, _ f: Frame) {
        guard let paint = f.bottom_paint, !paint.isEmpty else { return }
        let top = f.windows.map { $0.y + $0.h }.max() ?? 0
        // The first row is the command line, not part of the transient.
        let rect = CGRect(x: 0, y: CGFloat(top + 1) * cell.height, width: CGFloat(f.cols) * cell.width,
                          height: CGFloat(max(0, paint.count - 1)) * cell.height)
        paintFill(ctx, rect, palette.bg.blended(withFraction: 0.25, of: .black) ?? palette.bg)
        paintFill(ctx, CGRect(x: 0, y: rect.minY, width: rect.width, height: 0.5), palette.fg.withAlphaComponent(0.1))
        for (r, p) in paint.enumerated() where f.bottom.indices.contains(r) {
            for run in f.bottom[r] where (run.f ?? 0) & 8 != 0 {
                paintFill(ctx, paintRect(run.x, top + r, cellWidth(run.s)), palette.fg.withAlphaComponent(0.08))
            }
            drawMarks(ctx, p, runs: f.bottom[r], col: 0, row: top + r)
        }
    }

    /// Keep source text intact; mask only the count's parentheses in the GUI.
    /// Split at character boundaries so option styles stay on their cell spans.
    func paintedRuns(_ runs: [Run], _ paint: RowPaint?) -> [Run] {
        guard let paint else { return runs }
        var out: [Run] = []
        for run in runs {
            var x = run.x
            for ch in run.s {
                var part = run
                part.x = x; part.s = String(ch)
                if paint.marks?.contains(where: { $0.kind == "key" }) == true { part.f = (part.f ?? 0) & ~8 }
                for m in paint.marks ?? [] where x >= m.start && x < m.end {
                    if m.kind == "count" && ((x == m.start && ch == "(") || (x == m.end - 1 && ch == ")")) { part.s = " " }
                    if m.kind == "group" { part.f = (part.f ?? 0) | 1 }
                    if m.kind == "opt_on" { part.fg = 3 }
                    if m.kind == "opt_off" { part.fg = nil; part.f = (part.f ?? 0) | 16 }
                }
                out.append(part)
                x += cellWidth(String(ch))
            }
        }
        return out
    }
}
