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

    /// The sidebar filter: of `files` (relative to `root`), those whose
    /// name contains `query` (ignoring case; with a `/`, the relative path
    /// must contain it), as absolute paths together with every folder
    /// above them. At most `limit` matches.
    public static func filter(_ files: [String], query: String, under root: String, limit: Int = 2000) -> Set<String> {
        let q = query.lowercased(), base = root.hasSuffix("/") ? String(root.dropLast()) : root
        var out = Set<String>(), found = 0
        for f in files where found < limit {
            let hay = q.contains("/") ? f : String(f[(f.lastIndex(of: "/").map { f.index(after: $0) } ?? f.startIndex)...])
            guard hay.lowercased().contains(q) else { continue }
            found += 1
            var at = base
            for part in f.split(separator: "/") {
                at += "/" + part
                out.insert(at)
            }
        }
        return out
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

    /// Breadcrumbs for `path`: each component with its full path, relative
    /// to `root` when under it, else from `/`.
    public static func crumbs(_ path: String, root: String) -> [(name: String, path: String)] {
        let base = root.hasSuffix("/") ? root : root + "/"
        let under = path.hasPrefix(base)
        var at = under ? String(base.dropLast()) : ""
        return (under ? String(path.dropFirst(base.count)) : path).split(separator: "/").map {
            at += "/" + $0
            return (String($0), at)
        }
    }

    /// A VS Code–style file-type badge: short text in the language's color
    /// (0xRRGGBB); nil for types that use the plain symbol.
    public static func badge(for name: String) -> (text: String, rgb: UInt32)? {
        let lower = name.lowercased()
        switch lower {
        case "dockerfile": return ("🐳", 0x0DB7ED)
        case "makefile", "justfile": return ("M", 0x6D8086)
        case "cargo.toml", "cargo.lock": return ("RS", 0xDEA584)
        case "package.json": return ("npm", 0xCB3837)
        default: break
        }
        switch (lower as NSString).pathExtension {
        case "rs": return ("RS", 0xDEA584)
        case "php", "module", "install", "inc", "theme": return ("php", 0x8993BE)
        case "js", "mjs", "cjs": return ("JS", 0xF1E05A)
        case "jsx": return ("JSX", 0x61DAFB)
        case "ts": return ("TS", 0x3178C6)
        case "tsx": return ("TSX", 0x3178C6)
        case "py": return ("PY", 0x3572A5)
        case "rb": return ("RB", 0xCC342D)
        case "go": return ("GO", 0x00ADD8)
        case "c", "h": return ("C", 0x5E97D0)
        case "cpp", "cc", "hpp": return ("C++", 0xF34B7D)
        case "java": return ("J", 0xB07219)
        case "kt": return ("KT", 0xA97BFF)
        case "lua": return ("LUA", 0x6C7EF8)
        case "sh", "bash", "zsh", "fish": return ("$", 0x89E051)
        case "html", "htm": return ("<>", 0xE34C26)
        case "twig": return ("{%", 0xBACF29)
        case "css": return ("#", 0x563D7C)
        case "scss", "sass": return ("#", 0xC6538C)
        case "json": return ("{}", 0xCBCB41)
        case "yaml", "yml": return ("Y", 0xCB171E)
        case "toml": return ("T", 0x9C4221)
        case "xml", "plist": return ("<>", 0x0060AC)
        case "md", "markdown": return ("M↓", 0x519ABA)
        case "org": return ("org", 0x77AA99)
        case "sql": return ("SQL", 0xE38C00)
        case "lock": return ("🔒", 0x8B949E)
        default: return nil
        }
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
