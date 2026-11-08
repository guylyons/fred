import AppKit
import FredKit
import UniformTypeIdentifiers

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSMenuDelegate, NSToolbarDelegate {
    var window: NSWindow!
    var core: Core!
    var view: EditorView!
    /// The folder tree, once a project is open; before that the split shows a WelcomePane.
    var sidebar: Sidebar?
    /// The open project (the core's working directory); nil until one is chosen.
    private var project: URL?
    /// The last frame's cwd: a change (a chosen folder, `:cd`) is a new project.
    private var lastCwd: String?
    /// `Space w`'s number on the sidebar.
    private let badge = NSTextField(labelWithString: " 0 ")
    var split: NSSplitViewController!
    var activity: ActivityBar!
    /// VS Code's layout controls at the title bar's right: customize, then
    /// the sidebar, panel and chat toggles, lit while showing.
    private var layoutButtons: (custom: Pill, side: Pill, panel: Pill, chat: Pill)!
    private var layoutStack: NSStackView!
    /// The title bar's left: Fred's face and the project, as Nova shows them.
    private let projectTitle = ProjectTitle()
    private var minimapItem: NSMenuItem!
    private var glassItem: NSMenuItem!
    /// The window's backdrop and the editor's box (recolored by `applyTheme`).
    private let backdrop = EditorBackground()
    private let editorBox = EditorBackground()
    /// The title bar's strip: in Glass, the backdrop is clear, so this paints its tint.
    private let titleStrip = EditorBackground()
    /// View ▸ Glass Theme: the Ghostty look, translucent over a blurred desktop.
    private var glass = UserDefaults.standard.string(forKey: "FredTheme") == "glass"

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
            // VS Code's File menu, less workspaces, profiles, Share and Auto Save.
            menu("File", [
                item("New Text File", #selector(fileOp(_:)), "n", rep: "new"),
                item("New File…", #selector(newFile), "n", [.command, .option, .control]),
                item("New Window", #selector(newWindow), "n", [.command, .shift]),
                .separator(),
                item("Open…", #selector(openAny), "o"),
                item("Open Folder…", #selector(chooseFolder)),
                {
                    let i = item("Open Recent", nil)
                    i.submenu = NSMenu(title: "Open Recent")
                    i.submenu!.delegate = self
                    return i
                }(),
                .separator(),
                item("Save", #selector(macKey(_:)), "s", rep: "s"),
                item("Save As…", #selector(saveAs), "s", [.command, .shift]),
                item("Save All", #selector(fileOp(_:)), "s", [.command, .option], rep: "save_all"),
                .separator(),
                item("Revert File", #selector(fileOp(_:)), rep: "revert"),
                item("Close Editor", #selector(macKey(_:)), "w", rep: "w"),
                item("Close Window", #selector(quitCore), "w", [.command, .shift]),
            ]),
            // VS Code's Edit and Selection menus, less what Fred has no
            // counterpart for (multiple cursors, Replace in Files, Emmet,
            // block comments, Expand/Shrink Selection).
            menu("Edit", [
                item("Undo", #selector(macKey(_:)), "z", rep: "z"),
                item("Redo", #selector(macKey(_:)), "z", [.command, .shift], rep: "z", tag: 1),
                .separator(),
                item("Cut", #selector(macKey(_:)), "x", rep: "x"),
                item("Copy", #selector(macKey(_:)), "c", rep: "c"),
                item("Paste", #selector(paste), "v"),
                .separator(),
                item("Find", #selector(macKey(_:)), "f", rep: "f"),
                item("Replace", #selector(menuOp(_:)), "f", [.command, .option], rep: "replace"),
                .separator(),
                item("Find in Files", #selector(menuOp(_:)), "f", [.command, .shift], rep: "find_in_files"),
                .separator(),
                item("Toggle Line Comment", #selector(menuOp(_:)), "/", rep: "comment"),
            ]),
            menu("Selection", [
                item("Select All", #selector(macKey(_:)), "a", rep: "a"),
                .separator(),
                item("Copy Line Up", #selector(menuOp(_:)), Self.up, [.option, .shift], rep: "copy_up"),
                item("Copy Line Down", #selector(menuOp(_:)), Self.down, [.option, .shift], rep: "copy_down"),
                item("Move Line Up", #selector(menuOp(_:)), Self.up, [.option], rep: "move_up"),
                item("Move Line Down", #selector(menuOp(_:)), Self.down, [.option], rep: "move_down"),
                item("Duplicate Selection", #selector(menuOp(_:)), rep: "duplicate"),
                .separator(),
                item("Column Selection Mode", #selector(menuOp(_:)), rep: "column"),
            ]),
            menu("View", [
                {
                    let i = NSMenuItem(title: "Toggle Sidebar", action: #selector(toggleExplorer), keyEquivalent: "s")
                    i.keyEquivalentModifierMask = [.command, .control]
                    return i
                }(),
                .separator(),
                item("Bigger", #selector(zoom(_:)), "=", tag: 1),
                item("Smaller", #selector(zoom(_:)), "-", tag: -1),
                item("Actual Size", #selector(zoom(_:)), "0"),
                .separator(),
                item("Toggle Chat", #selector(toggleChat), "i", [.command, .control]),
                item("Toggle Terminal", #selector(panel(_:)), "`", [.control]),
                item("Maximize Terminal", #selector(panel(_:)), "~", [.control, .shift], tag: 1),
                .separator(),
                { minimapItem = item(view.showMinimap ? "Hide Minimap" : "Show Minimap", #selector(toggleMinimap), "m", [.command, .control]); return minimapItem }(),
                .separator(),
                { glassItem = item("Glass Theme", #selector(toggleGlass), "t", [.command, .control]); glassItem.state = glass ? .on : .off; return glassItem }(),
                item("Background Image…", #selector(chooseBackground)),
                item("Remove Background Image", #selector(removeBackground)),
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

    /// VS Code's Chat panel: Claude in a window on the right.
    @objc private func toggleChat() {
        view.transmit { .chat(seq: $0, op: "toggle", arg: nil) }
        window.makeFirstResponder(view)
    }

    private static let up = String(Character(UnicodeScalar(NSUpArrowFunctionKey)!))
    private static let down = String(Character(UnicodeScalar(NSDownArrowFunctionKey)!))

    @objc private func menuOp(_ s: NSMenuItem) {
        let op = s.representedObject as! String
        view.transmit(at: CACurrentMediaTime()) { .menu(seq: $0, op: op) }
    }

    @objc private func fileOp(_ s: NSMenuItem) {
        let op = s.representedObject as! String
        view.transmit { .file(seq: $0, op: op, path: nil) }
    }

    /// New File…: name it first, then open it empty (saved on the first write).
    @objc private func newFile() {
        let panel = NSSavePanel()
        panel.directoryURL = project
        panel.prompt = "Create File"
        panel.beginSheetModal(for: window) { [weak self] r in
            guard r == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated { self?.view.transmit { .open(seq: $0, path: url.path, preview: false) } }
        }
    }

    @objc private func saveAs() {
        let panel = NSSavePanel()
        let path = view.current?.windows.first(where: \.active)?.path
        panel.directoryURL = path.map { URL(fileURLWithPath: $0).deletingLastPathComponent() } ?? project
        panel.nameFieldStringValue = path.map { ($0 as NSString).lastPathComponent } ?? "Untitled"
        panel.beginSheetModal(for: window) { [weak self] r in
            guard r == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated { self?.view.transmit { .file(seq: $0, op: "save_as", path: url.path) } }
        }
    }

    /// Another Fred, as VS Code's New Window: its own process and project.
    @objc private func newWindow() {
        let c = NSWorkspace.OpenConfiguration()
        c.createsNewApplicationInstance = true
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: c)
    }

    /// Open…: a file opens in a tab, a folder becomes the project.
    @objc private func openAny() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.directoryURL = project
        panel.beginSheetModal(for: window) { [weak self] r in
            guard r == .OK else { return }
            MainActor.assumeIsolated {
                guard let self else { return }
                for url in panel.urls {
                    if url.hasDirectoryPath { self.open(folder: url) } else { self.view.transmit { .open(seq: $0, path: url.path, preview: false) } }
                }
            }
        }
    }

    /// Open Recent: the remembered project folders, newest first.
    func menuNeedsUpdate(_ menu: NSMenu) {
        guard menu.title == "Open Recent" else { return }
        menu.removeAllItems()
        for url in recentFolders() {
            let i = item((url.path as NSString).abbreviatingWithTildeInPath, #selector(openRecent(_:)), rep: url.path)
            menu.addItem(i)
        }
        if menu.items.isEmpty { menu.addItem(item("No Recent Folders", nil)) }
        menu.addItem(.separator())
        menu.addItem(item("Clear Recently Opened", #selector(clearRecent)))
    }

    @objc private func openRecent(_ s: NSMenuItem) { open(folder: URL(fileURLWithPath: s.representedObject as! String)) }

    @objc private func clearRecent() { UserDefaults.standard.removeObject(forKey: Self.recentKey) }

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

    /// View ▸ Background Image…: a picture behind the whole window, seen
    /// through translucent panes (Ghostty's background-image).
    private var backgroundImage = UserDefaults.standard.string(forKey: "FredBackgroundImage").flatMap(NSImage.init(contentsOfFile:))

    @objc private func chooseBackground() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        guard panel.runModal() == .OK, let url = panel.url, let image = NSImage(contentsOf: url) else { return }
        UserDefaults.standard.set(url.path, forKey: "FredBackgroundImage")
        backgroundImage = image
        applyTheme()
        installSidebar(project)
    }

    @objc private func removeBackground() {
        UserDefaults.standard.removeObject(forKey: "FredBackgroundImage")
        backgroundImage = nil
        applyTheme()
        installSidebar(project)
    }

    @objc private func toggleGlass() {
        glass.toggle()
        UserDefaults.standard.set(glass ? "glass" : "default", forKey: "FredTheme")
        glassItem.state = glass ? .on : .off
        applyTheme()
        installSidebar(project) // the sidebar takes its color when built
    }

    /// The two themes: solid Material Darker, or Glass, where only the
    /// bottom layer of each region paints the 90% tint so it isn't doubled.
    private func applyTheme() {
        let image = backgroundImage
        var p = glass || image != nil ? Palette.glass : .materialDark
        if image != nil { p.bg = p.bg.withAlphaComponent(0.88) }
        view.palette = p
        let chrome = p.bg.blended(withFraction: 0.12, of: .black)!
        backdrop.color = p.clear ? .clear : chrome
        backdrop.image = image
        activity?.tint = p.clear ? chrome : .clear
        titleStrip.color = p.clear ? chrome : .clear
        (split?.splitView as? DividerSplit)?.color = chrome.blended(withFraction: 0.3, of: .black)!
        editorBox.color = p.bg
        window.isOpaque = !glass
        window.backgroundColor = glass ? .clear : .windowBackgroundColor
        setBlur(window, radius: glass ? 20 : 0)
        func redraw(_ v: NSView) { v.needsDisplay = true; v.subviews.forEach(redraw) }
        window.contentView.map(redraw)
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
        // A file or folder argument opens its folder as the project; none
        // opens on the splash screen with no project, as VS Code does.
        let arg = pathArgument()
        var isDir: ObjCBool = false
        if let arg { FileManager.default.fileExists(atPath: arg.path, isDirectory: &isDir) }
        // Symlinks resolved: the core reports canonical paths.
        project = arg.map { a in
            let dir = isDir.boolValue ? a : a.deletingLastPathComponent()
            // A file inside a repository opens the repository, as `code` in a checkout does.
            var d = dir
            while d.path != "/" && !FileManager.default.fileExists(atPath: d.appendingPathComponent(".git").path) { d.deleteLastPathComponent() }
            return URL(fileURLWithPath: FileTree.canonical((isDir.boolValue || d.path == "/" ? dir : d).path))
        }
        let cwd = project ?? FileManager.default.homeDirectoryForCurrentUser

        let exe = ProcessInfo.processInfo.environment["FRED_CORE"].map { URL(fileURLWithPath: $0) }
            ?? Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("fred-core")
        core = Core(executable: exe, args: ["--gui"] + (arg.map { [$0.path] } ?? []), cwd: cwd)

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
        view.onResize = { [core, unowned view] cols, rows, margin in
            core!.send(.resize(cols: cols, rows: rows, margin: margin, aspect: Int((view.cell.height / view.cell.width * 100).rounded())))
        }
        view.reveal = { [unowned self] in self.sidebar?.revealInTree($0) }
        core.onFrame = { [unowned self] f in self.frame(f) }
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
        let split = split(editor: view)
        self.split = split
        // The title strip shows the theme color under it, not tinted glass.
        window.titlebarAppearsTransparent = true
        // The tab and breadcrumbs name the file; the title (kept for Mission
        // Control and the Window menu) would sit over the plain sidebar pane.
        window.titleVisibility = .hidden
        let content = withActivityBar(split)
        installToolbar()
        applyTheme()
        installSidebar(project)
        if let project { remember(project) }
        // FRED_KEYS=… types keys once the core is up: screenshots of a state without focus.
        if let keys = ProcessInfo.processInfo.environment["FRED_KEYS"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [unowned self] in run(keys) }
        }
        let saved = UserDefaults.standard.object(forKey: "NSSplitView Subview Frames FredSidebar") != nil
        split.splitView.autosaveName = "FredSidebar"
        if !saved { split.splitView.setPosition(240, ofDividerAt: 0) }
        window.contentViewController = content
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

    private func frame(_ f: Frame) {
        view.apply(f)
        if window.title != f.title { window.title = f.title }
        projectTitle.update(project: project, branch: f.windows.first(where: \.active)?.status.branch)
        if let cwd = f.cwd, cwd != lastCwd {
            // The first frame is where we started; a later change is a new project.
            if lastCwd != nil { setProject(URL(fileURLWithPath: cwd)) }
            lastCwd = cwd
        }
        sidebar?.follow(f.windows.first(where: \.active)?.path)
        sidebar?.showZap(f.zap_nav ?? [])
        if f.focus_nav == true { focusSidebar() }
        if f.open_folder == true { chooseFolder() }
        showBadge(f.picking == true && !split.splitViewItems[0].isCollapsed)
        refreshLayoutButtons()
    }

    private func refreshLayoutButtons() {
        guard let b = layoutButtons else { return }
        b.side.lit = !split.splitViewItems[0].isCollapsed
        b.panel.lit = view.current?.windows.contains { $0.term == true } ?? false
        b.chat.lit = view.current?.windows.contains { $0.chat != nil } ?? false
    }

    /// Customize Layout: the parts to show, as VS Code's menu lists them.
    private func customizeLayout(_ from: NSView) {
        refreshLayoutButtons()
        let m = NSMenu()
        for (title, on, action) in [
            ("Primary Side Bar", layoutButtons.side.lit, #selector(toggleExplorer)),
            ("Panel", layoutButtons.panel.lit, #selector(panel(_:))),
            ("Secondary Side Bar (Chat)", layoutButtons.chat.lit, #selector(toggleChat)),
            ("Minimap", view.showMinimap, #selector(toggleMinimap)),
            ("Glass Theme", glass, #selector(toggleGlass)),
        ] {
            let i = item(title, action)
            i.state = on ? .on : .off
            m.addItem(i)
        }
        m.popUp(positioning: nil, at: NSPoint(x: 0, y: from.bounds.maxY + 4), in: from)
    }

    /// Open Folder…: the chosen folder becomes the core's working directory
    /// (`cd`); the next frame's cwd brings the new sidebar.
    @objc private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.prompt = "Open"
        panel.message = "Choose a project folder: it becomes Fred's working directory."
        panel.beginSheetModal(for: window) { [weak self] r in
            guard r == .OK, let url = panel.url else { return }
            MainActor.assumeIsolated { self?.open(folder: url) }
        }
    }

    private func open(folder: URL) {
        view.transmit { .cd(seq: $0, path: folder.path) }
        window.makeFirstResponder(view)
    }

    private func setProject(_ url: URL) {
        guard url != project else { return }
        project = url
        view.root = url.path
        remember(url)
        installSidebar(url)
    }

    private static let recentKey = "FredRecentFolders"

    private func recentFolders() -> [URL] {
        (UserDefaults.standard.stringArray(forKey: Self.recentKey) ?? [])
            .filter { FileManager.default.fileExists(atPath: $0) }.map { URL(fileURLWithPath: $0) }
    }

    private func remember(_ url: URL) {
        let list = [url.path] + recentFolders().map(\.path).filter { $0 != url.path }
        UserDefaults.standard.set(Array(list.prefix(9)), forKey: Self.recentKey)
    }

    /// The sidebar for `root`: its folder tree, or with no project the
    /// welcome pane. Collapsed stays collapsed.
    private func installSidebar(_ root: URL?) {
        let bg = view.palette.bg.blended(withFraction: 0.12, of: .black)!
        let toEditor = { [unowned self] in self.window.makeFirstResponder(self.view); return }
        let vc: NSViewController
        sidebar?.stop()
        sidebar = nil
        if let root {
            let tree = Sidebar(root: root)
            tree.background = bg
            tree.open = { [unowned self] url, preview in
                self.view.transmit { .open(seq: $0, path: url.path, preview: preview) }
                toEditor()
            }
            tree.navChanged = { [unowned self] rows in self.core.send(.nav(rows: rows)) }
            tree.leave = toEditor
            tree.close = { [unowned self] in self.closeSidebar() }
            tree.forward = { [unowned self] in self.run($0) }
            sidebar = tree
            vc = tree
        } else {
            let pane = WelcomePane()
            pane.background = bg
            pane.recent = recentFolders()
            pane.choose = { [unowned self] in self.chooseFolder() }
            pane.openRecent = { [unowned self] in self.open(folder: $0) }
            pane.leave = toEditor
            pane.close = { [unowned self] in self.closeSidebar() }
            vc = pane
        }
        // A plain pane: a sidebar item wraps it in system glass that would hide
        // a background image or the Glass theme.
        let item = NSSplitViewItem(viewController: vc)
        item.minimumThickness = 140
        item.maximumThickness = 600
        item.canCollapse = true
        let old = split.splitViewItems.count > 1 ? split.splitViewItems[0] : nil
        let width = old.map { $0.viewController.view.frame.width } ?? 0
        item.isCollapsed = old?.isCollapsed ?? false
        if let old { split.removeSplitViewItem(old) }
        split.insertSplitViewItem(item, at: 0)
        if width > 0, !item.isCollapsed { split.splitView.setPosition(width, ofDividerAt: 0) }
        badge.removeFromSuperview()
        activity?.refresh()
    }

    /// `Space w`'s number for the sidebar, like the core's on each window.
    private func showBadge(_ on: Bool) {
        let v = split.splitViewItems[0].viewController.view
        guard on else { badge.isHidden = true; return }
        if badge.superview !== v {
            badge.font = .monospacedSystemFont(ofSize: 22, weight: .bold)
            badge.textColor = .systemYellow
            badge.wantsLayer = true
            badge.layer?.cornerRadius = 8
            badge.layer?.borderWidth = 2
            badge.layer?.borderColor = NSColor.systemYellow.cgColor
            badge.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.6).cgColor
            badge.translatesAutoresizingMaskIntoConstraints = false
            v.addSubview(badge)
            NSLayoutConstraint.activate([badge.centerXAnchor.constraint(equalTo: v.centerXAnchor),
                                         badge.centerYAnchor.constraint(equalTo: v.centerYAnchor)])
        }
        badge.isHidden = false
    }

    private func focusSidebar() {
        showSidebar()
        if let sidebar { sidebar.focusTree() } else { (split.splitViewItems[0].viewController as? WelcomePane)?.focus() }
    }

    /// q or CTRL-W c in the sidebar: it closes like a window (Space w 0,
    /// CTRL-W h from the leftmost window or ⌃⌘S bring it back).
    private func closeSidebar() {
        let side = split.splitViewItems[0]
        if !side.isCollapsed { side.animator().isCollapsed = true }
        window.makeFirstResponder(view)
        activity.refresh()
    }

    @objc private func toggleExplorer() {
        let side = split.splitViewItems[0]
        side.animator().isCollapsed.toggle()
        activity.refresh()
        refreshLayoutButtons()
    }

    private func showSidebar() {
        let side = split.splitViewItems[0]
        if side.isCollapsed { side.animator().isCollapsed = false; activity.refresh() }
    }

    /// The activity bar at the window's left edge, then the sidebar and editor.
    private func withActivityBar(_ split: NSSplitViewController) -> NSViewController {
        let c = NSViewController()
        // Solid like the sidebar: no wallpaper showing through.
        let glass = backdrop
        c.view = glass
        c.addChild(split)
        let bar = ActivityBar(top: [
            .init(symbol: "doc.on.doc", tip: "Explorer (⌃⌘S)", run: { [unowned self] in self.toggleExplorer() },
                  on: { [unowned self] in !self.split.splitViewItems[0].isCollapsed }),
            .init(symbol: "magnifyingglass", tip: "Grep (Space g)", run: { [unowned self] in self.run("<Space>g") }),
            .init(symbol: "doc.text.magnifyingglass", tip: "Find a File (Space p)", run: { [unowned self] in self.run("<Space>p") }),
            .init(symbol: "arrow.triangle.branch", tip: "Magit Status (Space m s)", run: { [unowned self] in self.run("<Space>ms") }),
            .init(symbol: "folder", tip: "Dired This Directory (Space -)", run: { [unowned self] in self.run("<Space>-") }),
            .init(symbol: "terminal", tip: "Toggle Terminal (⌃`); ⌃⇧` Maximize", run: { [unowned self] in
                self.view.transmit { .panel(seq: $0, op: "toggle") }
                self.window.makeFirstResponder(self.view)
            }),
            .init(symbol: "sparkles", tip: "Toggle Chat (⌃⌘I)", run: { [unowned self] in self.toggleChat() }),
        ], bottom: [
            .init(symbol: "map", tip: "Minimap (⌃⌘M)", run: { [unowned self] in self.toggleMinimap() },
                  on: { [unowned self] in self.view.showMinimap }),
            .init(symbol: "questionmark.circle", tip: "Help: Search Commands (Space h)", run: { [unowned self] in self.run("<Space>h") }),
        ])
        activity = bar
        for v in [titleStrip, bar, split.view] { v.translatesAutoresizingMaskIntoConstraints = false; glass.addSubview(v) }
        NSLayoutConstraint.activate([
            titleStrip.topAnchor.constraint(equalTo: glass.topAnchor),
            titleStrip.leadingAnchor.constraint(equalTo: glass.leadingAnchor),
            titleStrip.trailingAnchor.constraint(equalTo: glass.trailingAnchor),
            titleStrip.bottomAnchor.constraint(equalTo: glass.safeAreaLayoutGuide.topAnchor),
            bar.leadingAnchor.constraint(equalTo: glass.leadingAnchor),
            bar.topAnchor.constraint(equalTo: glass.safeAreaLayoutGuide.topAnchor), // under the title bar
            bar.bottomAnchor.constraint(equalTo: glass.bottomAnchor),
            bar.widthAnchor.constraint(equalToConstant: 48),
            split.view.leadingAnchor.constraint(equalTo: bar.trailingAnchor),
            split.view.topAnchor.constraint(equalTo: glass.safeAreaLayoutGuide.topAnchor),
            split.view.bottomAnchor.constraint(equalTo: glass.bottomAnchor),
            split.view.trailingAnchor.constraint(equalTo: glass.trailingAnchor),
        ])
        let icon = { (symbol: String, tip: String, run: @escaping () -> Void) -> Pill in
            let p = Pill(title: "")
            p.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tip)
            p.toolTip = tip
            p.square = true
            p.onClick = run
            return p
        }
        let custom = icon("square.grid.2x2", "Customize Layout…", {})
        custom.onClick = { [unowned self, unowned custom] in self.customizeLayout(custom) }
        let side = icon("rectangle.lefthalf.inset.filled", "Toggle Primary Side Bar (⌃⌘S)") { [unowned self] in
            self.toggleExplorer()
        }
        let panel = icon("rectangle.bottomhalf.inset.filled", "Toggle Panel (⌃`)") { [unowned self] in
            self.view.transmit { .panel(seq: $0, op: "toggle") }
            self.window.makeFirstResponder(self.view)
        }
        let chat = icon("rectangle.righthalf.inset.filled", "Toggle Secondary Side Bar (⌃⌘I)") { [unowned self] in self.toggleChat() }
        layoutButtons = (custom, side, panel, chat)
        let buttons = NSStackView(views: [custom, side, panel, chat])
        buttons.spacing = 2
        layoutStack = buttons // the title bar's right (installToolbar)
        glass.frame = NSRect(x: 0, y: 0, width: 1140, height: 640)
        return c
    }

    // MARK: - Title bar

    private static let projectItem = NSToolbarItem.Identifier("FredProject")
    private static let layoutItem = NSToolbarItem.Identifier("FredLayout")

    /// Nova's title bar: a unified toolbar strip with the traffic lights,
    /// Fred's face and the project on the left, the layout buttons right.
    private func installToolbar() {
        let t = NSToolbar(identifier: "FredTitleBar")
        t.delegate = self
        t.displayMode = .iconOnly
        t.allowsUserCustomization = false
        window.toolbar = t
        window.toolbarStyle = .unified
        window.titlebarSeparatorStyle = .none
        projectTitle.projectMenu = { [unowned self] in self.projectMenu() }
        projectTitle.update(project: project, branch: nil)
    }

    func toolbarDefaultItemIdentifiers(_ t: NSToolbar) -> [NSToolbarItem.Identifier] {
        [Self.projectItem, .flexibleSpace, Self.layoutItem]
    }

    func toolbarAllowedItemIdentifiers(_ t: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(t)
    }

    func toolbar(_ t: NSToolbar, itemForItemIdentifier id: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        let i = NSToolbarItem(itemIdentifier: id)
        i.view = id == Self.projectItem ? projectTitle : layoutStack
        i.isBordered = false
        i.label = id == Self.projectItem ? "Project" : "Layout"
        return i
    }

    /// A click on the project: recent folders, then Open Folder….
    private func projectMenu() -> NSMenu {
        let m = NSMenu()
        for url in recentFolders() where url != project {
            let i = item(url.lastPathComponent, #selector(openRecent(_:)), rep: url.path)
            i.image = NSWorkspace.shared.icon(forFile: url.path)
            i.image?.size = NSSize(width: 16, height: 16)
            i.toolTip = url.path
            m.addItem(i)
        }
        if !m.items.isEmpty { m.addItem(.separator()) }
        m.addItem(item("Open Folder…", #selector(chooseFolder)))
        if let project {
            m.addItem(item("Reveal in Finder", #selector(revealProject(_:)), rep: project.path))
        }
        return m
    }

    @objc private func revealProject(_ s: NSMenuItem) {
        guard let p = s.representedObject as? String else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: p)])
    }

    /// The editor, with room for the sidebar (`installSidebar`) beside it; its width is remembered.
    private func split(editor: EditorView) -> NSSplitViewController {
        let split = NSSplitViewController()
        split.splitView = DividerSplit()
        split.splitView.isVertical = true
        split.splitView.dividerStyle = .thin
        let content = NSViewController()
        let box = editorBox
        editor.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(editor)
        // Under the title bar: the tabs are the first row.
        NSLayoutConstraint.activate([
            editor.topAnchor.constraint(equalTo: box.topAnchor),
            editor.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            editor.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            editor.bottomAnchor.constraint(equalTo: box.bottomAnchor),
        ])
        content.view = box
        split.addSplitViewItem(NSSplitViewItem(viewController: content))
        split.view.frame = NSRect(x: 0, y: 0, width: 1140, height: 640)
        return split
    }

    /// The close button asks the core to `:qa`; its `exit` line closes us.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        core.send(.quit)
        return false
    }

    /// The first non-flag argument, if any.
    private func pathArgument() -> URL? {
        CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") })
            .map { URL(fileURLWithPath: $0).standardizedFileURL }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.appearance = NSAppearance(named: .darkAqua) // dark materials and label colors over the dark theme
let delegate = AppDelegate()
app.delegate = delegate
app.run()

/// The sidebar's edge: a hairline that doesn't show the backdrop through.
final class DividerSplit: NSSplitView {
    var color = NSColor.black { didSet { needsDisplay = true } }
    override var dividerColor: NSColor { color }
    override func drawDivider(in rect: NSRect) { color.setFill(); rect.fill() }
}

/// Opaque theme background behind the editor (and under the toolbar).
class EditorBackground: NSView {
    var color = NSColor.black { didSet { needsDisplay = true } }
    /// Drawn under `color`, filling the view (cropped, not stretched).
    var image: NSImage? { didSet { needsDisplay = true } }
    override var isOpaque: Bool { color.alphaComponent == 1 || image != nil }
    override func draw(_ r: NSRect) {
        if let image, image.size.width > 0, image.size.height > 0 {
            let s = max(bounds.width / image.size.width, bounds.height / image.size.height)
            let size = NSSize(width: image.size.width * s, height: image.size.height * s)
            image.draw(in: NSRect(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2,
                                  width: size.width, height: size.height))
        }
        color.setFill()
        r.fill(using: .sourceOver)
    }
}

/// Ghostty's background-blur: the private window-server call it uses (looked
/// up at run time, so a missing symbol only loses the blur).
@MainActor func setBlur(_ w: NSWindow, radius: Int) {
    typealias Conn = @convention(c) () -> UnsafeMutableRawPointer?
    typealias Set = @convention(c) (UnsafeMutableRawPointer?, Int, Int) -> Int32
    guard let h = dlopen(nil, RTLD_NOW),
          let c = dlsym(h, "CGSDefaultConnectionForThread"),
          let f = dlsym(h, "CGSSetWindowBackgroundBlurRadius") else { return }
    _ = unsafeBitCast(f, to: Set.self)(unsafeBitCast(c, to: Conn.self)(), w.windowNumber, radius)
}
