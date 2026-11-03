import AppKit
import FredKit

/// File-type icons for tabs and the sidebar: a colored badge for known
/// languages (FileTree.badge), else the row's SF Symbol.
@MainActor
enum FileIcon {
    private static var cache: [String: NSImage] = [:]

    static func image(for name: String, isDir: Bool, size: CGFloat = 14) -> NSImage {
        let badge = isDir ? nil : FileTree.badge(for: name)
        let key = "\(badge?.text ?? FileTree.symbol(for: name, isDir: isDir))|\(badge?.rgb ?? 0)|\(size)"
        if let i = cache[key] { return i }
        let img: NSImage
        if let (text, rgb) = badge {
            let color = NSColor(red: CGFloat(rgb >> 16 & 0xFF) / 255, green: CGFloat(rgb >> 8 & 0xFF) / 255,
                                blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
            img = NSImage(size: NSSize(width: size, height: size), flipped: false) { r in
                // Longer labels shrink to fit the square.
                var pt = size * (text.count <= 1 ? 0.8 : text.count == 2 ? 0.62 : 0.48)
                var s: NSAttributedString
                repeat {
                    s = NSAttributedString(string: text, attributes: [
                        .font: NSFont.systemFont(ofSize: pt, weight: .heavy), .foregroundColor: color,
                    ])
                    pt -= 0.5
                } while s.size().width > r.width && pt > 4
                let z = s.size()
                s.draw(at: NSPoint(x: r.midX - z.width / 2, y: r.midY - z.height / 2))
                return true
            }
        } else {
            let sym = NSImage(systemSymbolName: FileTree.symbol(for: name, isDir: isDir), accessibilityDescription: nil)?
                .withSymbolConfiguration(.init(pointSize: size * 0.8, weight: .regular)) ?? NSImage()
            let tint: NSColor = isDir ? .controlAccentColor.withAlphaComponent(0.85) : .secondaryLabelColor
            img = NSImage(size: NSSize(width: size, height: size), flipped: false) { r in
                let z = sym.size
                let at = NSRect(x: r.midX - z.width / 2, y: r.midY - z.height / 2, width: z.width, height: z.height)
                sym.draw(in: at)
                tint.set()
                at.fill(using: .sourceAtop)
                return true
            }
        }
        cache[key] = img
        return img
    }
}
