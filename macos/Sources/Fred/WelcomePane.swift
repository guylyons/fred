import AppKit

/// The sidebar before a project is chosen, as VS Code's Explorer with no
/// folder: an Open Folder button and the recent folders.
@MainActor
final class WelcomePane: NSViewController {
    var background = NSColor.black
    var recent: [URL] = []
    /// Open Folder…: the folder panel.
    var choose: () -> Void = {}
    /// A recent folder, clicked.
    var openRecent: (URL) -> Void = { _ in }
    var leave: () -> Void = {}
    var close: () -> Void = {}

    override func loadView() {
        let v = WelcomeView(frame: NSRect(x: 0, y: 0, width: 240, height: 600))
        v.color = background
        v.pane = self
        let header = NSTextField(labelWithString: "NO FOLDER OPENED")
        header.font = .systemFont(ofSize: 12, weight: .semibold)
        header.textColor = .secondaryLabelColor
        let note = NSTextField(wrappingLabelWithString: "You have not yet opened a folder. The one you open becomes Fred's working directory.")
        note.font = .systemFont(ofSize: 13)
        note.textColor = .secondaryLabelColor
        let button = NSButton(title: "Open Folder…", target: self, action: #selector(chooseClicked))
        button.bezelStyle = .push
        button.controlSize = .large
        let stack = NSStackView(views: [header, note, button])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        if !recent.isEmpty {
            let title = NSTextField(labelWithString: "RECENT")
            title.font = .systemFont(ofSize: 12, weight: .semibold)
            title.textColor = .secondaryLabelColor
            stack.addArrangedSubview(title)
            stack.setCustomSpacing(18, after: button)
            for (i, url) in recent.enumerated() {
                let b = NSButton(title: url.lastPathComponent, target: self, action: #selector(recentClicked(_:)))
                b.isBordered = false
                b.font = .systemFont(ofSize: 14)
                b.contentTintColor = .linkColor
                b.toolTip = url.path
                b.tag = i
                stack.addArrangedSubview(b)
            }
        }
        stack.translatesAutoresizingMaskIntoConstraints = false
        v.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: v.safeAreaLayoutGuide.topAnchor, constant: 6),
            stack.leadingAnchor.constraint(equalTo: v.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: v.trailingAnchor, constant: -12),
            note.widthAnchor.constraint(lessThanOrEqualTo: v.widthAnchor, constant: -28),
        ])
        view = v
    }

    /// The pane takes the keyboard: Enter or o opens the folder panel, 1-9 a
    /// recent folder, Esc goes back to the editor, q closes the sidebar.
    func focus() { view.window?.makeFirstResponder(view) }

    fileprivate func key(_ e: NSEvent) -> Bool {
        guard let c = e.charactersIgnoringModifiers?.first else { return false }
        switch c {
        case "\r", "o": choose()
        case "q": close()
        case "\u{1b}": leave()
        case "1"..."9" where Int(String(c))! <= recent.count: openRecent(recent[Int(String(c))! - 1])
        default: return false
        }
        return true
    }

    @objc private func chooseClicked() { choose() }
    @objc private func recentClicked(_ b: NSButton) { openRecent(recent[b.tag]) }
}

private final class WelcomeView: EditorBackground {
    weak var pane: WelcomePane?
    override var acceptsFirstResponder: Bool { true }
    override func keyDown(with e: NSEvent) { if pane?.key(e) != true { super.keyDown(with: e) } }
}
