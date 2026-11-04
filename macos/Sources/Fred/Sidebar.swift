import AppKit
import CoreServices
import FredKit

/// One row: a file or folder. Children load on first expand.
@MainActor
final class Node {
    let url: URL
    let isDir: Bool
    var children: [Node]?
    var ignored = false
    /// New File/Folder not yet on disk: its row is being named.
    var pending = false

    init(_ url: URL, isDir: Bool) { self.url = url; self.isDir = isDir }
    var name: String { url.lastPathComponent }

    /// (Re)list the folder, keeping the Node objects (and so expansion and
    /// selection) of entries still there.
    func load() {
        let old = Dictionary((children ?? []).map { ($0.name, $0) }, uniquingKeysWith: { a, _ in a })
        let urls = (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: [.isDirectoryKey])) ?? []
        let all = urls.map {
            (name: $0.lastPathComponent, isDir: (try? $0.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true)
        }
        let cargo = all.contains { $0.name == "Cargo.toml" }
        let entries = all.filter { !FileTree.isHidden($0.name, isDir: $0.isDir, cargo: cargo) }
        children = FileTree.sorted(entries).map { e in
            if let n = old[e.name], n.isDir == e.isDir, !n.pending { return n }
            return Node(url.appendingPathComponent(e.name, isDirectory: e.isDir), isDir: e.isDir)
        }
    }
}

/// The folder tree on the left (Sublime's sidebar), in the split view's
/// glass sidebar item.
@MainActor
final class Sidebar: NSViewController, NSOutlineViewDataSource, NSOutlineViewDelegate, NSMenuDelegate, NSTextFieldDelegate {
    let root: Node
    /// Opens a file in the active Fred window; `true` as a preview tab.
    var open: (URL, Bool) -> Void = { _, _ in }
    /// Esc in the tree: back to the editor.
    var leave: () -> Void = {}
    /// q or CTRL-W c in the tree: hide the sidebar, as closing a window.
    var close: () -> Void = {}
    /// Space and `:` in the tree: these keys, typed in the editor.
    var forward: (String) -> Void = { _ in }
    private let outline = SidebarOutline()
    private var stream: FSEventStreamRef?
    /// The active window's file, as last revealed.
    private var current: String?
    private var editing: Node?
    /// The visible rows changed: (name, path) top to bottom, for zap.
    var navChanged: ([(name: String, path: String)]) -> Void = { _ in }
    private var navRows: [Int] = []
    private var navSent: [String] = []
    private var zapViews: [NSView] = []

    init(root: URL) {
        self.root = Node(URL(fileURLWithPath: FileTree.canonical(root.path)), isDir: true)
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Solid, not the sidebar glass (which takes on the wallpaper's color).
    var background = NSColor.black

    override func loadView() {
        let v = EditorBackground(frame: NSRect(x: 0, y: 0, width: 240, height: 600))
        v.color = background
        let header = NSTextField(labelWithString: root.name.uppercased())
        header.font = .systemFont(ofSize: 12, weight: .semibold)
        header.textColor = .secondaryLabelColor
        header.lineBreakMode = .byTruncatingTail

        let col = NSTableColumn(identifier: .init("name"))
        outline.addTableColumn(col)
        outline.outlineTableColumn = col
        outline.headerView = nil
        outline.style = .sourceList
        outline.rowSizeStyle = .custom
        outline.rowHeight = 24 // VS Code's compact rows
        outline.dataSource = self
        outline.delegate = self
        // Files drag into the editor (and out, as copies, to other apps).
        outline.setDraggingSourceOperationMask(.copy, forLocal: false)
        outline.target = self
        outline.action = #selector(clicked)
        outline.doubleAction = #selector(doubleClicked)
        outline.activate = { [weak self] in self?.activateSelected() }
        outline.leave = { [weak self] in self?.leave() }
        outline.close = { [weak self] in self?.close() }
        outline.forward = { [weak self] in self?.forward($0) }
        outline.menu = NSMenu()
        outline.menu!.delegate = self
        let scroll = NSScrollView()
        scroll.documentView = outline
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true

        for s in [header, scroll] as [NSView] {
            s.translatesAutoresizingMaskIntoConstraints = false
            v.addSubview(s)
        }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: v.safeAreaLayoutGuide.topAnchor, constant: 6),
            header.leadingAnchor.constraint(equalTo: v.leadingAnchor, constant: 16),
            header.trailingAnchor.constraint(lessThanOrEqualTo: v.trailingAnchor, constant: -8),
            scroll.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 4),
            scroll.leadingAnchor.constraint(equalTo: v.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: v.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: v.bottomAnchor),
        ])
        view = v
        watch()
    }

    // MARK: - Data

    private func kids(_ item: Any?) -> [Node] {
        let n = item as? Node ?? root
        if n.children == nil { load(n) }
        return n.children ?? []
    }

    /// List `n`, then dim its gitignored entries once Git says which.
    private func load(_ n: Node) {
        n.load()
        let names = (n.children ?? []).filter { !$0.pending }.map(\.name)
        let dir = n.url
        guard !names.isEmpty else { return }
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let ignored = gitIgnored(names, in: dir)
            guard !ignored.isEmpty else { return }
            DispatchQueue.main.async {
                guard let self else { return }
                for c in n.children ?? [] where ignored.contains(c.name) && !c.ignored {
                    c.ignored = true
                    self.outline.reloadItem(c)
                }
            }
        }
    }

    func outlineView(_ o: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int { kids(item).count }
    func outlineView(_ o: NSOutlineView, child i: Int, ofItem item: Any?) -> Any { kids(item)[i] }
    func outlineView(_ o: NSOutlineView, isItemExpandable item: Any) -> Bool { (item as! Node).isDir }

    /// A file row drags as its URL: EditorView drops it into a window or a split.
    func outlineView(_ o: NSOutlineView, pasteboardWriterForItem item: Any) -> (any NSPasteboardWriting)? {
        guard let n = item as? Node, !n.pending else { return nil } // a folder drops as dired
        return n.url as NSURL
    }

    func outlineView(_ o: NSOutlineView, viewFor col: NSTableColumn?, item: Any) -> NSView? {
        let n = item as! Node
        let id = NSUserInterfaceItemIdentifier("cell")
        let cell = o.makeView(withIdentifier: id, owner: nil) as? NSTableCellView ?? {
            let c = NSTableCellView()
            c.identifier = id
            let image = NSImageView(), text = NSTextField(labelWithString: "")
            text.font = .systemFont(ofSize: 13)
            text.lineBreakMode = .byTruncatingMiddle
            text.delegate = self
            for s in [image, text] as [NSView] {
                s.translatesAutoresizingMaskIntoConstraints = false
                c.addSubview(s)
            }
            NSLayoutConstraint.activate([
                image.leadingAnchor.constraint(equalTo: c.leadingAnchor, constant: 2),
                image.centerYAnchor.constraint(equalTo: c.centerYAnchor),
                image.widthAnchor.constraint(equalToConstant: 16),
                text.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 5),
                text.trailingAnchor.constraint(equalTo: c.trailingAnchor, constant: -2),
                text.centerYAnchor.constraint(equalTo: c.centerYAnchor),
            ])
            c.imageView = image
            c.textField = text
            return c
        }()
        cell.imageView!.image = FileIcon.image(for: n.name, isDir: n.isDir, size: 16)
        cell.textField!.stringValue = n.name
        cell.textField!.textColor = n.ignored ? .tertiaryLabelColor : .labelColor
        cell.textField!.isEditable = false
        return cell
    }

    // MARK: - Opening

    @objc private func clicked() {
        guard let n = outline.item(atRow: outline.clickedRow) as? Node else { return }
        // The disclosure triangle has already toggled it.
        if n.isDir, let e = NSApp.currentEvent,
           outline.frameOfOutlineCell(atRow: outline.clickedRow).contains(outline.convert(e.locationInWindow, from: nil)) { return }
        activate(n, preview: true)
    }

    /// The first click already toggled a folder (and previewed a file); a
    /// double-click opens the file for good.
    @objc private func doubleClicked() {
        guard let n = outline.item(atRow: outline.clickedRow) as? Node, !n.isDir else { return }
        activate(n, preview: false)
    }

    private func activateSelected() {
        if let n = outline.item(atRow: outline.selectedRow) as? Node { activate(n) }
    }

    private func activate(_ n: Node, preview: Bool = false) {
        guard !n.pending else { return }
        if !n.isDir { open(n.url, preview); return }
        if outline.isItemExpanded(n) { outline.collapseItem(n) } else { outline.expandItem(n) }
    }

    /// Select and reveal the active window's file, expanding its folders;
    /// keyboard focus stays where it is.
    func follow(_ path: String?) {
        sendNav()
        guard path != current else { return }
        current = path
        select(path)
    }

    /// Tell the core the rows on screen, when they change.
    private func sendNav() {
        guard isViewLoaded else { return }
        let r = outline.rows(in: outline.visibleRect)
        let rows = (r.location..<r.location + r.length).filter { (outline.item(atRow: $0) as? Node)?.pending == false }
        let nodes = rows.compactMap { outline.item(atRow: $0) as? Node }
        let paths = nodes.map(\.url.path)
        guard paths != navSent else { return }
        navSent = paths
        navRows = rows
        navChanged(nodes.map { ($0.name, $0.url.path) })
    }

    /// Zap's labels over the sidebar rows: the typed letters in yellow, the label in magenta.
    func showZap(_ labels: [NavLabel]) {
        guard isViewLoaded, !(labels.isEmpty && zapViews.isEmpty) else { return }
        zapViews.forEach { $0.removeFromSuperview() }
        zapViews = []
        for l in labels where l.row < navRows.count {
            guard let cell = outline.view(atColumn: 0, row: navRows[l.row], makeIfNecessary: false) as? NSTableCellView,
                  let tf = cell.textField else { continue }
            let name = tf.stringValue, font = tf.font ?? .systemFont(ofSize: 13)
            func width(_ n: Int) -> CGFloat {
                NSAttributedString(string: String(name.prefix(n)), attributes: [.font: font]).size().width
            }
            let x = tf.frame.minX + 2 + width(l.at), h = tf.frame.height
            if l.typed > 0 {
                let hi = NSView(frame: NSRect(x: x, y: tf.frame.minY, width: width(l.at + l.typed) - width(l.at), height: h))
                hi.wantsLayer = true
                hi.layer?.backgroundColor = NSColor.systemYellow.withAlphaComponent(0.45).cgColor
                cell.addSubview(hi)
                zapViews.append(hi)
            }
            let tag = NSTextField(labelWithString: l.label)
            tag.font = .monospacedSystemFont(ofSize: 12, weight: .bold)
            tag.textColor = .black
            tag.drawsBackground = true
            tag.backgroundColor = .systemPink
            tag.sizeToFit()
            tag.frame.origin = NSPoint(x: x + width(l.at + l.typed) - width(l.at), y: tf.frame.midY - tag.frame.height / 2)
            cell.addSubview(tag)
            zapViews.append(tag)
        }
    }

    /// Reveal in Sidebar (a tab's menu): select `path` without making it
    /// the followed file, so the next frame doesn't undo it.
    func revealInTree(_ path: String) { select(path) }

    /// The tree takes the keyboard, on the current file's row (else the first).
    func focusTree() {
        if outline.selectedRow < 0, outline.numberOfRows > 0 {
            outline.selectRowIndexes([0], byExtendingSelection: false)
        }
        outline.scrollRowToVisible(max(0, outline.selectedRow))
        view.window?.makeFirstResponder(outline)
    }

    private func select(_ path: String?) {
        guard let raw = path, case let path = FileTree.canonical(raw),
              let dirs = FileTree.ancestors(of: path, under: root.url.path) else {
            outline.deselectAll(nil)
            return
        }
        var at = root
        for d in dirs + [path] {
            guard let n = kids(at === root ? nil : at).first(where: { $0.url.path == d }) else { return }
            if n.isDir { outline.expandItem(n) }
            at = n
        }
        let row = outline.row(forItem: at)
        guard row >= 0 else { return }
        outline.selectRowIndexes([row], byExtendingSelection: false)
        outline.scrollRowToVisible(row)
    }

    // MARK: - Live updates

    /// Stop watching (before this sidebar is replaced): FSEvents holds it unretained.
    func stop() {
        guard let s = stream else { return }
        FSEventStreamStop(s)
        FSEventStreamInvalidate(s)
        FSEventStreamRelease(s)
        stream = nil
    }

    private func watch() {
        var ctx = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(),
                                       retain: nil, release: nil, copyDescription: nil)
        let cb: FSEventStreamCallback = { _, info, count, paths, _, _ in
            let me = Unmanaged<Sidebar>.fromOpaque(info!).takeUnretainedValue()
            let dirs = unsafeBitCast(paths, to: NSArray.self) as! [String]
            MainActor.assumeIsolated { me.changed(Array(dirs.prefix(count))) }
        }
        guard let s = FSEventStreamCreate(nil, cb, &ctx, [root.url.path] as CFArray,
                                          FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.3,
                                          FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes)) else { return }
        FSEventStreamSetDispatchQueue(s, .main)
        FSEventStreamStart(s)
        stream = s
    }

    /// Reload the loaded folders FSEvents names (it names folders).
    private func changed(_ dirs: [String]) {
        guard editing == nil else { return } // don't pull the row out from under the name field
        for d in Set(dirs.map { FileTree.canonical($0.hasSuffix("/") && $0.count > 1 ? String($0.dropLast()) : $0) }) {
            if let n = loaded(d) { reload(n) }
        }
    }

    /// The loaded folder at `path`.
    private func loaded(_ path: String) -> Node? {
        guard let n = existing(path), n.isDir, n.children != nil else { return nil }
        return n
    }

    /// The node at `path` among those already listed (nothing is loaded).
    private func existing(_ path: String) -> Node? {
        if path == root.url.path { return root }
        guard let dirs = FileTree.ancestors(of: path, under: root.url.path) else { return nil }
        var at = root
        for d in dirs + [path] {
            guard let n = at.children?.first(where: { $0.url.path == d }) else { return nil }
            at = n
        }
        return at
    }

    private func reload(_ n: Node) {
        load(n)
        outline.reloadItem(n === root ? nil : n, reloadChildren: true)
        // Re-select the current file (its Node may be new) if its row is
        // showing; a folder you collapsed stays collapsed.
        guard let c = current, let node = existing(FileTree.canonical(c)) else { return }
        let row = outline.row(forItem: node)
        if row >= 0 { outline.selectRowIndexes([row], byExtendingSelection: false) }
    }

    // MARK: - Context menu

    /// The clicked row, or the root for empty space.
    private var target: Node { outline.item(atRow: outline.clickedRow) as? Node ?? root }

    func menuNeedsUpdate(_ m: NSMenu) {
        m.removeAllItems()
        func add(_ t: String, _ a: Selector) { m.addItem(withTitle: t, action: a, keyEquivalent: "").target = self }
        add("New File…", #selector(newFile))
        add("New Folder…", #selector(newFolder))
        if target !== root {
            m.addItem(.separator())
            add("Rename…", #selector(rename))
            add("Delete", #selector(delete))
        }
        m.addItem(.separator())
        add("Reveal in Finder", #selector(reveal))
        add("Copy Path", #selector(copyPath))
        add("Copy Relative Path", #selector(copyRelativePath))
    }

    @objc private func newFile() { create(dir: false) }
    @objc private func newFolder() { create(dir: true) }

    /// A pending row in the clicked folder (or the clicked file's folder), named inline.
    private func create(dir: Bool) {
        var parent = target
        if !parent.isDir { parent = outline.parent(forItem: parent) as? Node ?? root }
        if parent !== root { outline.expandItem(parent) }
        let n = Node(parent.url.appendingPathComponent(dir ? "untitled folder" : "untitled"), isDir: dir)
        n.pending = true
        let kids = kids(parent === root ? nil : parent)
        // Folders go first, files after the folders.
        let at = dir ? 0 : (kids.firstIndex { !$0.isDir } ?? kids.count)
        parent.children!.insert(n, at: at)
        outline.insertItems(at: [at], inParent: parent === root ? nil : parent)
        edit(n)
    }

    @objc private func rename() { edit(target) }

    private func edit(_ n: Node) {
        let row = outline.row(forItem: n)
        guard row >= 0, let cell = outline.view(atColumn: 0, row: row, makeIfNecessary: true) as? NSTableCellView,
              let field = cell.textField else { return }
        editing = n
        outline.scrollRowToVisible(row)
        field.isEditable = true
        view.window?.makeFirstResponder(field)
        // Select the name without its extension, as Finder does.
        let stem = (n.name as NSString).deletingPathExtension
        field.currentEditor()?.selectedRange = NSRange(location: 0, length: n.isDir || stem.isEmpty ? n.name.utf16.count : stem.utf16.count)
    }

    /// Esc cancels New/Rename: nothing is created or renamed.
    func control(_ c: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
        guard sel == #selector(NSResponder.cancelOperation(_:)), let n = editing, let field = c as? NSTextField else { return false }
        editing = nil // controlTextDidEndEditing from the abort does nothing
        field.abortEditing()
        field.isEditable = false
        field.stringValue = n.name
        let parentNode = outline.parent(forItem: n) as? Node ?? root
        if n.pending { parentNode.children?.removeAll { $0 === n } }
        reload(parentNode)
        view.window?.makeFirstResponder(outline)
        return true
    }

    func controlTextDidEndEditing(_ note: Notification) {
        guard let n = editing, let field = note.object as? NSTextField else { return }
        editing = nil
        field.isEditable = false
        let name = field.stringValue.trimmingCharacters(in: .whitespaces)
        let parentNode = outline.parent(forItem: n) as? Node ?? root
        let to = parentNode.url.appendingPathComponent(name, isDirectory: n.isDir)
        var opened = false
        defer {
            reload(parentNode)
            // Back to the tree after Enter, unless a file opened (the editor
            // has focus) or you clicked somewhere else to end the edit.
            let w = view.window, r = w?.firstResponder
            if !opened, r == nil || r === w || r === field { w?.makeFirstResponder(outline) }
        }
        guard !name.isEmpty, !name.contains("/"), n.pending || name != n.name else {
            if n.pending { parentNode.children?.removeAll { $0 === n } }
            return
        }
        do {
            if n.pending {
                parentNode.children?.removeAll { $0 === n }
                if n.isDir {
                    try FileManager.default.createDirectory(at: to, withIntermediateDirectories: false)
                } else {
                    // Never clobber an existing file.
                    guard !FileManager.default.fileExists(atPath: to.path) else { throw CocoaError(.fileWriteFileExists) }
                    guard FileManager.default.createFile(atPath: to.path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
                    open(to, false)
                    opened = true
                }
            } else {
                try FileManager.default.moveItem(at: n.url, to: to)
                // ponytail: only the active window follows a rename (via `open`); other buffers keep the old path.
                if let c = current, FileTree.canonical(c) == n.url.path { open(to, false); opened = true }
            }
        } catch {
            NSAlert(error: error).runModal()
        }
    }

    @objc private func delete() {
        let n = target
        NSWorkspace.shared.recycle([n.url]) { [weak self] _, err in
            DispatchQueue.main.async {
                guard let self else { return }
                if let err { NSAlert(error: err).runModal() }
                self.reload(self.outline.parent(forItem: n) as? Node ?? self.root)
            }
        }
    }

    @objc private func reveal() { NSWorkspace.shared.activateFileViewerSelecting([target.url]) }

    @objc private func copyPath() { copy(target.url.path) }

    @objc private func copyRelativePath() {
        let p = target.url.path, r = root.url.path + "/"
        copy(p.hasPrefix(r) ? String(p.dropFirst(r.count)) : ".")
    }

    private func copy(_ s: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(s, forType: .string)
    }
}

/// The tree as a vim window: j/k, h/l (collapse or parent / expand or
/// open), gg/G, Ctrl-D/U, Enter or o opens; Esc, Ctrl-L or CTRL-W l back
/// to the editor; q or CTRL-W c/q closes it; Space and : are the editor's.
final class SidebarOutline: NSOutlineView {
    var activate: () -> Void = {}
    var leave: () -> Void = {}
    var close: () -> Void = {}
    var forward: (String) -> Void = { _ in }
    private var pending: Character?

    override func keyDown(with e: NSEvent) {
        let ctrl = e.modifierFlags.contains(.control)
        guard let c = e.charactersIgnoringModifiers?.first else { return super.keyDown(with: e) }
        let was = pending
        pending = nil
        if was == "w" {
            if "cq".contains(c) { close() } else if "lwWp".contains(c) { leave() }
            return
        }
        if ctrl {
            switch c {
            case "w": pending = "w"
            case "l": leave()
            case "d": go(selectedRow + halfPage)
            case "u": go(selectedRow - halfPage)
            default: super.keyDown(with: e)
            }
            return
        }
        switch c {
        case _ where [36, 76].contains(e.keyCode): activate()
        case _ where e.keyCode == 53: leave()
        case "o": activate()
        case "j": go(selectedRow + 1)
        case "k": go(selectedRow - 1)
        case "G": go(numberOfRows - 1)
        case "g": if was == "g" { go(0) } else { pending = "g" }
        case "l": right()
        case "h": left()
        case "q": close()
        case " ": forward("<Space>")
        case ":": forward(":")
        default: super.keyDown(with: e)
        }
    }

    private var halfPage: Int { max(1, Int(visibleRect.height / max(1, rowHeight)) / 2) }

    private func go(_ row: Int) {
        guard numberOfRows > 0 else { return }
        let r = min(max(0, row), numberOfRows - 1)
        selectRowIndexes([r], byExtendingSelection: false)
        scrollRowToVisible(r)
    }

    /// A folder opens, an open one steps into its first entry; a file opens.
    private func right() {
        guard let n = item(atRow: selectedRow) as? Node else { return }
        if !n.isDir { return activate() }
        if isItemExpanded(n) { go(selectedRow + 1) } else { expandItem(n) }
    }

    /// An open folder closes; otherwise up to the parent folder.
    private func left() {
        guard let n = item(atRow: selectedRow) else { return }
        if isItemExpanded(n) { return collapseItem(n) }
        if let p = parent(forItem: n) { go(row(forItem: p)) }
    }
}

/// Which of `names` (entries of `dir`) Git ignores; empty outside a repo.
private func gitIgnored(_ names: [String], in dir: URL) -> Set<String> {
    let p = Process(), input = Pipe(), output = Pipe()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/git")
    p.arguments = ["check-ignore", "-z", "--stdin"]
    p.currentDirectoryURL = dir
    p.standardInput = input
    p.standardOutput = output
    p.standardError = FileHandle.nullDevice
    guard (try? p.run()) != nil else { return [] }
    // Written from another thread: Git answers as it reads, and a full
    // stdout pipe would otherwise block both of us.
    let w = input.fileHandleForWriting, names = Data(names.joined(separator: "\0").utf8 + [0])
    Thread {
        try? w.write(contentsOf: names)
        try? w.close()
    }.start()
    let data = output.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    return Set(data.split(separator: 0).map { String(decoding: $0, as: UTF8.self) })
}
