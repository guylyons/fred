import AppKit
import FredKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSToolbarDelegate {
    var window: NSWindow!
    var core: Core!
    var view: EditorView!
    var sidebar: Sidebar!
    var split: NSSplitViewController!
    var activity: ActivityBar!
    private var minimapItem: NSMenuItem!

    // MARK: - Menus

    private func item(_ title: String, _ action: Selector?, _ key: String = "",
                      _ mods: NSEvent.ModifierFlags = .command, rep: String? = nil, tag: Int = 0) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.keyEquivalentModifierMask = mods
        i.target = action == nil || action == #selector(NSApplication.orderFrontStandardAboutPanel(_:)) || action == #selector(NSApplication.hide(_:)) ? nil : self
        i.representedObject = rep
        i.tag = tag
        return i
    }

    private func menu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let m = NSMenu(title: title)
        items.forEach(m.addItem)
        let top = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        top.submenu = m
        return top
    }

    private func buildMenus() {
        let bar = NSMenu()
        let windowMenu = menu("Window", [
            NSMenuItem(title: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m"),
            NSMenuItem(title: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: ""),
            .separator(),
            // Shift is in the characters: Cmd-} is Cmd-Shift-].
            item("Select Next Tab", #selector(selectTab(_:)), "}", tag: 100),
            item("Select Previous Tab", #selector(selectTab(_:)), "{", tag: -100),
        ] + (1...9).map { item("Select Tab \($0)", #selector(selectTab(_:)), "\($0)", tag: $0) })
        [
            menu("Fred", [
                item("About Fred", #selector(NSApplication.orderFrontStandardAboutPanel(_:))),
                .separator(),
                item("Settings…", nil, ","), // disabled in Phase 1
                .separator(),
                item("Hide Fred", #selector(NSApplication.hide(_:)), "h"),
                .separator(),
                item("Quit Fred", #selector(quitCore), "q"),
            ]),
            menu("File", [
                item("Open…", nil, "o"), // disabled in Phase 1
                item("Save", #selector(macKey(_:)), "s", rep: "s"),
                item("Close Tab", #selector(macKey(_:)), "w", rep: "w"),
            ]),
            menu("Edit", [
                item("Undo", #selector(macKey(_:)), "z", rep: "z"),
                item("Redo", #selector(macKey(_:)), "z", [.command, .shift], rep: "z", tag: 1),
                .separator(),
                item("Cut", #selector(macKey(_:)), "x", rep: "x"),
                item("Copy", #selector(macKey(_:)), "c", rep: "c"),
                item("Paste", #selector(paste), "v"),
                item("Select All", #selector(macKey(_:)), "a", rep: "a"),
                .separator(),
                item("Find", #selector(macKey(_:)), "f", rep: "f"),
            ]),
            menu("View", [
                {
                    // The split view controller answers, and retitles it Show/Hide Sidebar.
                    let i = NSMenuItem(title: "Show Sidebar", action: #selector(NSSplitViewController.toggleSidebar(_:)), keyEquivalent: "s")
                    i.keyEquivalentModifierMask = [.command, .control]
                    return i
                }(),
                .separator(),
                item("Bigger", #selector(zoom(_:)), "=", tag: 1),
                item("Smaller", #selector(zoom(_:)), "-", tag: -1),
                item("Actual Size", #selector(zoom(_:)), "0"),
                .separator(),
                item("Toggle Terminal", #selector(panel(_:)), "`", [.control]),
                item("Maximize Terminal", #selector(panel(_:)), "~", [.control, .shift], tag: 1),
                .separator(),
                { minimapItem = item(view.showMinimap ? "Hide Minimap" : "Show Minimap", #selector(toggleMinimap), "m", [.command, .control]); return minimapItem }(),
            ]),
            windowMenu,
        ].forEach(bar.addItem)
        NSApp.mainMenu = bar
        NSApp.windowsMenu = windowMenu.submenu
    }

    /// Cmd-Q asks the core to `:qa`, as the close button does: with unsaved
    /// changes the core stays and says so, however often it is pressed.
    @objc private func quitCore() {
        guard let core, !core.exited else { NSApp.terminate(nil); return }
        core.send(.quit)
    }

    private var quitAsked: Date?

    /// A system quit (Dock, logout) asks the core to `:qa`; its `exit` line
    /// ends the app. A second request within 2 s quits anyway, for a hung
    /// core (which then sees EOF and saves its swap). Cmd-Q doesn't come
    /// here (`quitCore`).
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let core, !core.exited else { return .terminateNow }
        if let t = quitAsked, Date().timeIntervalSince(t) < 2 { return .terminateNow }
        quitAsked = Date()
        core.send(.quit)
        return .terminateCancel
    }

    @objc private func macKey(_ s: NSMenuItem) {
        let k = s.representedObject as! String, shift = s.tag == 1
        view.transmit(at: CACurrentMediaTime()) { .mac(seq: $0, key: k, shift: shift) }
    }

    /// tag 1...9: that tab; 100 / -100: next / previous, wrapping.
    @objc private func selectTab(_ s: NSMenuItem) {
        guard let w = view.current?.windows.first(where: \.active), let id = w.id, let tabs = w.tabs, !tabs.isEmpty else { return }
        let cur = tabs.firstIndex(where: \.current) ?? 0
        let i = abs(s.tag) == 100 ? (cur + s.tag / 100 + tabs.count) % tabs.count : s.tag - 1
        guard i < tabs.count, i != cur else { return }
        view.transmit { .tab(seq: $0, win: id, op: "select", index: i, to: nil) }
    }

    @objc private func paste() {
        guard let t = NSPasteboard.general.string(forType: .string) else { return }
        view.transmit { .paste(seq: $0, text: t) }
    }

    @objc private func toggleMinimap() {
        view.showMinimap.toggle()
        UserDefaults.standard.set(view.showMinimap, forKey: "FredMinimap")
        minimapItem.title = view.showMinimap ? "Hide Minimap" : "Show Minimap"
        activity.refresh()
    }

    /// VS Code's terminal panel: tag 0 toggles, 1 maximizes (or restores).
    @objc private func panel(_ s: NSMenuItem) {
        let op = s.tag == 1 ? "maximize" : "toggle"
        view.transmit { .panel(seq: $0, op: op) }
        window.makeFirstResponder(view)
    }

    /// An activity bar command: leader keys, from Normal mode.
    private func run(_ keys: String) {
        view.transmit { .keys(seq: $0, keys: "<Esc>" + keys) }
        window.makeFirstResponder(view)
    }

    /// tag 1 bigger, -1 smaller, 0 actual size; 9–36 pt.
    @objc private func zoom(_ s: NSMenuItem) {
        let size = s.tag == 0 ? EditorView.defaultSize : min(36, max(9, view.font.pointSize + CGFloat(s.tag)))
        view.font = EditorView.editorFont(size)
    }

    func applicationDidFinishLaunching(_ n: Notification) {
        let path = pathToOpen()
        var isDir: ObjCBool = false
        FileManager.default.fileExists(atPath: path.path, isDirectory: &isDir)
        // Symlinks resolved: the core reports canonical paths.
        let cwd = URL(fileURLWithPath: FileTree.canonical((isDir.boolValue ? path : path.deletingLastPathComponent()).path))

        let exe = ProcessInfo.processInfo.environment["FRED_CORE"].map { URL(fileURLWithPath: $0) }
            ?? Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("fred-core")
        core = Core(executable: exe, args: ["--gui", path.path], cwd: cwd)

        let view = EditorView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
        self.view = view
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1140, height: 640),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                          backing: .buffered, defer: false)
        window.title = "Fred"
        window.delegate = self
        view.send = { [core] in core!.send($0) }
        view.root = cwd.path
        view.showMinimap = UserDefaults.standard.object(forKey: "FredMinimap") as? Bool ?? true
        view.onResize = { [core] cols, rows, margin in core!.send(.resize(cols: cols, rows: rows, margin: margin)) }
        let sidebar = Sidebar(root: cwd)
        sidebar.background = view.palette.bg.blended(withFraction: 0.12, of: .black)!
        sidebar.open = { url, preview in
            view.transmit { .open(seq: $0, path: url.path, preview: preview) }
            view.window?.makeFirstResponder(view)
        }
        view.reveal = { sidebar.revealInTree($0) }
        sidebar.navChanged = { [core] rows in core!.send(.nav(rows: rows)) }
        sidebar.leave = { view.window?.makeFirstResponder(view) }
        self.sidebar = sidebar
        core.onFrame = { [window] f in
            view.apply(f)
            if window!.title != f.title { window!.title = f.title }
            sidebar.follow(f.windows.first(where: \.active)?.path)
            sidebar.showZap(f.zap_nav ?? [])
            if f.focus_nav == true { self.showSidebar(); sidebar.focusTree() }
        }
        core.onMinimap = { view.applyMinimap($0) }
        core.onExit = { [window] code, tail in
            if code != 0 {
                let alert = NSAlert()
                alert.messageText = "fred exited with code \(code)"
                alert.informativeText = tail
                alert.runModal()
            }
            window!.close()
            NSApp.terminate(nil)
        }
        let bar = NSToolbar(identifier: "main")
        bar.delegate = self
        bar.displayMode = .iconOnly
        window.toolbar = bar
        window.toolbarStyle = .unified
        // The title strip shows the theme color under it, not tinted glass.
        window.titlebarAppearsTransparent = true
        let split = split(sidebar: sidebar, editor: view)
        self.split = split
        window.contentViewController = withActivityBar(split)
        // Most of the screen: room for the splash logo at full size.
        let screen = NSScreen.main?.visibleFrame.size ?? NSSize(width: 1425, height: 800)
        window.setContentSize(NSSize(width: max(1140, screen.width * 0.8), height: max(640, screen.height * 0.85)))
        window.makeFirstResponder(view)
        self.view = view
        buildMenus()
        window.center()
        // FRED_BACKGROUND=1: open behind (for screenshots while you work).
        if ProcessInfo.processInfo.environment["FRED_BACKGROUND"] == "1" {
            window.orderFront(nil)
        } else {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate()
        }
    }

    private func showSidebar() {
        let side = split.splitViewItems[0]
        if side.isCollapsed { side.animator().isCollapsed = false; activity.refresh() }
    }

    /// The activity bar at the window's left edge, then the sidebar and editor.
    private func withActivityBar(_ split: NSSplitViewController) -> NSViewController {
        let c = NSViewController()
        // Solid like the sidebar: no wallpaper showing through.
        let glass = EditorBackground()
        glass.color = view.palette.bg.blended(withFraction: 0.12, of: .black)!
        c.view = glass
        c.addChild(split)
        let bar = ActivityBar(top: [
            .init(symbol: "doc.on.doc", tip: "Explorer (⌃⌘S)", run: { [unowned self] in self.split.toggleSidebar(nil) },
                  on: { [unowned self] in !self.split.splitViewItems[0].isCollapsed }),
            .init(symbol: "magnifyingglass", tip: "Grep (Space g)", run: { [unowned self] in self.run("<Space>g") }),
            .init(symbol: "doc.text.magnifyingglass", tip: "Find a File (Space p)", run: { [unowned self] in self.run("<Space>p") }),
            .init(symbol: "arrow.triangle.branch", tip: "Magit Status (Space m s)", run: { [unowned self] in self.run("<Space>ms") }),
            .init(symbol: "folder", tip: "Dired This Directory (Space -)", run: { [unowned self] in self.run("<Space>-") }),
            .init(symbol: "terminal", tip: "Toggle Terminal (⌃`); ⌃⇧` Maximize", run: { [unowned self] in
                self.view.transmit { .panel(seq: $0, op: "toggle") }
                self.window.makeFirstResponder(self.view)
            }),
            .init(symbol: "sparkles", tip: "Claude (Space c c)", run: { [unowned self] in self.run("<Space>cc") }),
        ], bottom: [
            .init(symbol: "map", tip: "Minimap (⌃⌘M)", run: { [unowned self] in self.toggleMinimap() },
                  on: { [unowned self] in self.view.showMinimap }),
            .init(symbol: "questionmark.circle", tip: "Help: Search Commands (Space h)", run: { [unowned self] in self.run("<Space>h") }),
        ])
        activity = bar
        for v in [bar, split.view] { v.translatesAutoresizingMaskIntoConstraints = false; glass.addSubview(v) }
        NSLayoutConstraint.activate([
            bar.leadingAnchor.constraint(equalTo: glass.leadingAnchor),
            bar.topAnchor.constraint(equalTo: glass.topAnchor), // under the traffic lights too: one shade
            bar.bottomAnchor.constraint(equalTo: glass.bottomAnchor),
            bar.widthAnchor.constraint(equalToConstant: 48),
            split.view.leadingAnchor.constraint(equalTo: bar.trailingAnchor),
            split.view.topAnchor.constraint(equalTo: glass.topAnchor),
            split.view.bottomAnchor.constraint(equalTo: glass.bottomAnchor),
            split.view.trailingAnchor.constraint(equalTo: glass.trailingAnchor),
        ])
        glass.frame = NSRect(x: 0, y: 0, width: 1140, height: 640)
        return c
    }

    /// The folder sidebar (glass, width remembered) beside the editor.
    private func split(sidebar: Sidebar, editor: EditorView) -> NSSplitViewController {
        let split = NSSplitViewController()
        let side = NSSplitViewItem(sidebarWithViewController: sidebar)
        side.minimumThickness = 140
        side.maximumThickness = 600
        side.canCollapse = true
        let content = NSViewController()
        let box = EditorBackground()
        box.color = editor.palette.bg
        editor.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(editor)
        // Below the toolbar; the box keeps the theme color under it.
        NSLayoutConstraint.activate([
            editor.topAnchor.constraint(equalTo: box.safeAreaLayoutGuide.topAnchor),
            editor.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            editor.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            editor.bottomAnchor.constraint(equalTo: box.bottomAnchor),
        ])
        content.view = box
        split.addSplitViewItem(side)
        split.addSplitViewItem(NSSplitViewItem(viewController: content))
        split.view.frame = NSRect(x: 0, y: 0, width: 1140, height: 640)
        let key = "NSSplitView Subview Frames FredSidebar"
        let saved = UserDefaults.standard.object(forKey: key) != nil
        split.splitView.autosaveName = "FredSidebar"
        if !saved { split.splitView.setPosition(240, ofDividerAt: 0) }
        return split
    }

    func toolbarDefaultItemIdentifiers(_ t: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.sidebarTrackingSeparator] // the activity bar's Explorer button toggles the sidebar
    }

    /// Required, or AppKit drops the delegate; the system items need nothing from us.
    func toolbar(_ t: NSToolbar, itemForItemIdentifier id: NSToolbarItem.Identifier, willBeInsertedIntoToolbar: Bool) -> NSToolbarItem? { nil }

    func toolbarAllowedItemIdentifiers(_ t: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(t)
    }

    /// The close button asks the core to `:qa`; its `exit` line closes us.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        core.send(.quit)
        return false
    }

    /// The first non-flag argument, else the home folder (a folder opens on the splash screen).
    private func pathToOpen() -> URL {
        CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") })
            .map { URL(fileURLWithPath: $0).standardizedFileURL } ?? FileManager.default.homeDirectoryForCurrentUser
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.appearance = NSAppearance(named: .darkAqua) // dark materials and label colors over the dark theme
let delegate = AppDelegate()
app.delegate = delegate
app.run()

/// Opaque theme background behind the editor (and under the toolbar).
final class EditorBackground: NSView {
    var color = NSColor.black
    override var isOpaque: Bool { true }
    override func draw(_ r: NSRect) { color.setFill(); r.fill() }
}
