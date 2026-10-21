import Foundation
import FredKit
import Synchronization

/// The `fred --gui` helper process, spoken to in JSON lines.
@MainActor
final class Core {
    var onFrame: (Frame) -> Void = { _ in }
    /// Exit code (-1 when the core died without an `exit` line) and the last
    /// 20 lines of its stderr.
    var onExit: (Int, String) -> Void = { _, _ in }

    private let process = Process()
    private let stdin = Pipe()
    private var exited = false

    init(executable: URL, args: [String], cwd: URL) {
        signal(SIGPIPE, SIG_IGN) // a dead core must not kill the app on write
        let out = Pipe(), err = Pipe()
        process.executableURL = executable
        process.arguments = args
        process.currentDirectoryURL = cwd
        process.standardInput = stdin
        process.standardOutput = out
        process.standardError = err

        let tail = Mutex<[String]>([])
        let errDone = DispatchSemaphore(value: 0)
        Thread {
            forEachLine(err.fileHandleForReading) { line in
                let s = String(decoding: line, as: UTF8.self)
                tail.withLock { $0.append(s); if $0.count > 20 { $0.removeFirst() } }
            }
            errDone.signal()
        }.start()
        Thread { [weak self] in
            forEachLine(out.fileHandleForReading) { line in
                do {
                    let msg = try Incoming.decode(line)
                    var t = ""
                    if case .exit = msg { t = tail.withLock { $0.joined(separator: "\n") } }
                    DispatchQueue.main.async { self?.handle(msg, tail: t) }
                } catch {
                    NSLog("fred: bad core line: %@", String(decoding: line, as: UTF8.self))
                }
            }
            errDone.wait()
            let t = tail.withLock { $0.joined(separator: "\n") }
            DispatchQueue.main.async { self?.handle(nil, tail: t) }
        }.start()

        do { try process.run() } catch {
            // ponytail: the reader threads stay blocked on the unused pipes; the app quits anyway.
            exited = true
            let msg = "\(executable.path): \(error.localizedDescription)"
            DispatchQueue.main.async { [weak self] in self?.onExit(-1, msg) }
        }
    }

    func send(_ m: Outgoing) {
        try? stdin.fileHandleForWriting.write(contentsOf: m.line())
    }

    /// `nil` is EOF on stdout.
    private func handle(_ msg: Incoming?, tail: String) {
        guard !exited else { return }
        switch msg {
        case .frame(let f): onFrame(f)
        case .exit(let code): exited = true; onExit(code, tail)
        case nil: exited = true; onExit(-1, tail)
        }
    }
}

/// Calls `body` with each "\n"-terminated line (without the newline) until EOF.
private func forEachLine(_ h: FileHandle, _ body: (Data) -> Void) {
    var buf = Data()
    while true {
        let chunk = h.availableData
        if chunk.isEmpty { break }
        buf.append(chunk)
        while let nl = buf.firstIndex(of: 10) {
            body(buf[buf.startIndex..<nl])
            buf.removeSubrange(buf.startIndex...nl)
        }
    }
    if !buf.isEmpty { body(buf) }
}
