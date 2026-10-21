import Foundation

/// Pure rules for the folder sidebar: what is hidden, the order, icons, reveal.
public enum FileTree {
    /// Never listed: `.git`, `.DS_Store`, and a `target` folder beside a
    /// `Cargo.toml` (`cargo`). Dotfiles like `.gitignore` and `.github` are
    /// listed, as in Sublime.
    public static func isHidden(_ name: String, isDir: Bool, cargo: Bool) -> Bool {
        name == ".git" || name == ".DS_Store" || (name == "target" && isDir && cargo)
    }

    /// The real path (symlinks resolved, so /tmp is /private/tmp, as the
    /// core reports it); `path` itself when it doesn't exist.
    public static func canonical(_ path: String) -> String {
        guard let p = realpath(path, nil) else { return path }
        defer { free(p) }
        return String(cString: p)
    }

    /// Folders first, then files, each case-insensitively by name.
    public static func sorted(_ entries: [(name: String, isDir: Bool)]) -> [(name: String, isDir: Bool)] {
        entries.sorted { a, b in
            if a.isDir != b.isDir { return a.isDir }
            let c = a.name.caseInsensitiveCompare(b.name)
            return c == .orderedSame ? a.name < b.name : c == .orderedAscending
        }
    }

    /// The folders to expand to reveal `path`: from `root`'s child down to
    /// `path`'s parent (empty for a file directly in `root`); nil when
    /// `path` isn't under `root`. Both standardized absolute paths.
    public static func ancestors(of path: String, under root: String) -> [String]? {
        let base = root.hasSuffix("/") ? root : root + "/"
        guard path.hasPrefix(base), path.count > base.count else { return nil }
        let parts = path.dropFirst(base.count).split(separator: "/").dropLast()
        var at = String(base.dropLast())
        return parts.map { at += "/" + $0; return at }
    }

    /// An SF Symbol for a row.
    public static func symbol(for name: String, isDir: Bool) -> String {
        if isDir { return "folder" }
        switch (name as NSString).pathExtension.lowercased() {
        case "swift": return "swift"
        case "png", "jpg", "jpeg", "gif", "svg", "webp", "heic", "ico": return "photo"
        case "json", "toml", "yaml", "yml", "lock", "plist": return "curlybraces"
        case "rs", "js", "ts", "tsx", "jsx", "py", "rb", "go", "c", "h", "cpp", "lua", "sh", "php", "twig", "css", "scss", "html", "java", "kt":
            return "chevron.left.forwardslash.chevron.right"
        case "md", "txt", "org", "csv", "": return "doc.text"
        default: return "doc"
        }
    }
}
