import AppKit
import FredKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSToolbarDelegate {
    var window: NSWindow!
    var core: Core!
    var view: EditorView!
    var sidebar: Sidebar!

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
        ])
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
                item("Close Window", #selector(macKey(_:)), "w", rep: "w"),
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
            ]),
            windowMenu,
        ].forEach(bar.addItem)
        NSApp.mainMenu = bar
        NSApp.windowsMenu = windowMenu.submenu
    }

    @objc private func quitCore() { NSApp.terminate(nil) }

    private var quitAsked: Date?

    /// Quit asks the core to `:qa`, as the close button does; its `exit`
    /// line ends the app. A second request within 2 s quits anyway, for a
    /// hung core (which then sees EOF and saves its swap).
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

    @objc private func paste() {
        guard let t = NSPasteboard.general.string(forType: .string) else { return }
        view.transmit { .paste(seq: $0, text: t) }
    }

    /// tag 1 bigger, -1 smaller, 0 actual size; 9–36 pt.
    @objc private func zoom(_ s: NSMenuItem) {
        let size = s.tag == 0 ? 13 : min(36, max(9, view.font.pointSize + CGFloat(s.tag)))
        view.font = .monospacedSystemFont(ofSize: size, weight: .regular)
    }

    func applicationDidFinishLaunching(_ n: Notification) {
        guard let path = pathToOpen() else { NSApp.terminate(nil); return }
        var isDir: ObjCBool = false
        FileManager.default.fileExists(atPath: path.path, isDirectory: &isDir)
        // Symlinks resolved: the core reports canonical paths.
        let cwd = URL(fileURLWithPath: FileTree.canonical((isDir.boolValue ? path : path.deletingLastPathComponent()).path))

        let exe = ProcessInfo.processInfo.environment["FRED_CORE"].map { URL(fileURLWithPath: $0) }
            ?? Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("fred-core")
        core = Core(executable: exe, args: ["--gui", path.path], cwd: cwd)

        let view = EditorView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1140, height: 640),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                          backing: .buffered, defer: false)
        window.title = "Fred"
        window.delegate = self
        view.send = { [core] in core!.send($0) }
        view.onResize = { [core] cols, rows in core!.send(.resize(cols: cols, rows: rows)) }
        let sidebar = Sidebar(root: cwd)
        sidebar.open = { url in view.transmit { .open(seq: $0, path: url.path) } ; view.window?.makeFirstResponder(view) }
        sidebar.leave = { view.window?.makeFirstResponder(view) }
        self.sidebar = sidebar
        core.onFrame = { [window] f in
            view.apply(f)
            if window!.title != f.title { window!.title = f.title }
            sidebar.follow(f.windows.first(where: \.active)?.path)
        }
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
        window.contentViewController = split(sidebar: sidebar, editor: view)
        window.setContentSize(NSSize(width: 1140, height: 640))
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
        [.toggleSidebar, .sidebarTrackingSeparator]
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

    /// The first non-flag argument, else an open panel (nil when cancelled).
    private func pathToOpen() -> URL? {
        if let arg = CommandLine.arguments.dropFirst().first(where: { !$0.hasPrefix("-") }) {
            return URL(fileURLWithPath: arg).standardizedFileURL
        }
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        return panel.runModal() == .OK ? panel.url : nil
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
