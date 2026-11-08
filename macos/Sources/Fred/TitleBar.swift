import AppKit
import FredKit

/// Nova's title bar, left of center: Fred's face, then the project folder
/// (its Finder icon, its name, the Git branch under it), between slashes.
/// Clicking the project lists recent folders and Open Folder….
final class ProjectTitle: NSView {
    private let face = PixelImage()
    private let folder = NSImageView()
    private let name = NSTextField(labelWithString: "")
    private let branch = NSTextField(labelWithString: "")
    private let project = NSStackView()
    /// The menu to show for a click on the project.
    var projectMenu: () -> NSMenu? = { nil }
    private var shown: (String?, String?) = ("-", "-")

    init() {
        super.init(frame: .zero)
        face.image = ProjectTitle.faceImage
        face.toolTip = "Fred"
        folder.imageScaling = .scaleProportionallyUpOrDown
        name.font = .systemFont(ofSize: 13, weight: .semibold)
        name.textColor = .labelColor
        name.lineBreakMode = .byTruncatingMiddle
        branch.font = .systemFont(ofSize: 11)
        branch.textColor = .secondaryLabelColor
        branch.lineBreakMode = .byTruncatingTail
        let lines = NSStackView(views: [name, branch])
        lines.orientation = .vertical
        lines.alignment = .leading
        lines.spacing = 0
        project.setViews([folder, lines], in: .leading)
        project.spacing = 6
        let row = NSStackView(views: [face, Slash(), project, Slash()])
        row.spacing = 8
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            face.widthAnchor.constraint(equalToConstant: 22),
            face.heightAnchor.constraint(equalToConstant: 26),
            folder.widthAnchor.constraint(equalToConstant: 22),
            folder.heightAnchor.constraint(equalToConstant: 22),
            name.widthAnchor.constraint(lessThanOrEqualToConstant: 260),
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.centerYAnchor.constraint(equalTo: centerYAnchor),
            heightAnchor.constraint(equalToConstant: 36),
        ])
        update(project: nil, branch: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The project folder (nil: none open yet) and the active window's branch.
    func update(project url: URL?, branch b: String?) {
        guard shown != (url?.path, b) else { return }
        shown = (url?.path, b)
        name.stringValue = url?.lastPathComponent ?? "No Folder Open"
        folder.image = url.map { NSWorkspace.shared.icon(forFile: $0.path) }
            ?? NSImage(systemSymbolName: "folder", accessibilityDescription: nil)
        branch.stringValue = b ?? ""
        branch.isHidden = b == nil
        toolTip = url?.path
    }

    override func mouseDown(with e: NSEvent) {
        guard project.frame.insetBy(dx: -4, dy: -4).contains(convert(e.locationInWindow, from: nil)),
              let m = projectMenu() else { return super.mouseDown(with: e) }
        m.popUp(positioning: nil, at: NSPoint(x: project.frame.minX, y: project.frame.minY - 4), in: self)
    }

    /// `dr-fred-head.png`: in the app's Resources, else the checkout's docs
    /// (a `swift run` build); else the app icon.
    static let faceImage: NSImage? = {
        let dev = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("../../../docs/dr-fred-head.png").standardizedFileURL
        let url = Bundle.main.url(forResource: "dr-fred-head", withExtension: "png") ?? dev
        return NSImage(contentsOf: url) ?? NSApp.applicationIconImage
    }()
}

/// Nova's separator: a thin slash, the bar's height.
private final class Slash: NSView {
    override var intrinsicContentSize: NSSize { NSSize(width: 10, height: 30) }
    override func draw(_ r: NSRect) {
        let p = NSBezierPath()
        p.move(to: NSPoint(x: bounds.minX + 1, y: bounds.minY + 2))
        p.line(to: NSPoint(x: bounds.maxX - 1, y: bounds.maxY - 2))
        p.lineWidth = 1
        NSColor.tertiaryLabelColor.setStroke()
        p.stroke()
    }
}

/// Pixel art, scaled up without smoothing and kept in proportion.
private final class PixelImage: NSView {
    var image: NSImage? { didSet { needsDisplay = true } }
    override func draw(_ r: NSRect) {
        guard let image, image.size.width > 0, image.size.height > 0 else { return }
        let s = min(bounds.width / image.size.width, bounds.height / image.size.height)
        let size = NSSize(width: image.size.width * s, height: image.size.height * s)
        NSGraphicsContext.current?.imageInterpolation = .none
        image.draw(in: NSRect(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2,
                              width: size.width, height: size.height))
    }
}
