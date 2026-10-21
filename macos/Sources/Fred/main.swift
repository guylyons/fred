import AppKit
import FredKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var window: NSWindow!
    var core: Core!

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
        view.onResize = { [core] cols, rows in core!.send(.resize(cols: cols, rows: rows)) }
        core.onFrame = { [window] f in
            view.apply(f)
            window!.title = f.title
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
let delegate = AppDelegate()
app.delegate = delegate
app.run()
