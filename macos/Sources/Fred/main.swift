import AppKit
import FredKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var window: NSWindow!
    var core: Core!
    var view: EditorView!

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
                item("Bigger", #selector(zoom(_:)), "=", tag: 1),
                item("Smaller", #selector(zoom(_:)), "-", tag: -1),
                item("Actual Size", #selector(zoom(_:)), "0"),
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
        let cwd = isDir.boolValue ? path : path.deletingLastPathComponent()

        let exe = ProcessInfo.processInfo.environment["FRED_CORE"].map { URL(fileURLWithPath: $0) }
            ?? Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("fred-core")
        core = Core(executable: exe, args: ["--gui", path.path], cwd: cwd)

        let view = EditorView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
        window = NSWindow(contentRect: view.frame, styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        window.title = "Fred"
        window.delegate = self
        view.send = { [core] in core!.send($0) }
        view.onResize = { [core] cols, rows in core!.send(.resize(cols: cols, rows: rows)) }
        core.onFrame = { [window] f in
            view.apply(f)
            if window!.title != f.title { window!.title = f.title }
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
        window.contentView = view
        window.makeFirstResponder(view)
        self.view = view
        buildMenus()
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate()
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
