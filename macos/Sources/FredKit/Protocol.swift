import Foundation

public struct Run: Codable, Equatable, Sendable {
    public var x: Int
    public var s: String
    public var fg: Int?
    public var bg: Int?
    public var f: Int?
}

/// Optional semantic hints; old cores omit them and old apps ignore them.
public struct RowPaint: Codable, Equatable, Sendable {
    public struct Role: Codable, Equatable, Sendable {
        public var kind: String
        public var open: Bool?
    }
    public struct Mark: Codable, Equatable, Sendable {
        public var start: Int
        public var end: Int
        public var kind: String
        public init(from decoder: Decoder) throws {
            var c = try decoder.unkeyedContainer()
            start = try c.decode(Int.self)
            end = try c.decode(Int.self)
            kind = try c.decode(String.self)
        }
        public func encode(to encoder: Encoder) throws {
            var c = encoder.unkeyedContainer()
            try c.encode(start); try c.encode(end); try c.encode(kind)
        }
    }
    public var role: Role?
    public var marks: [Mark]?
}

public struct Status: Codable, Equatable, Sendable {
    public var mode: String
    public var name: String
    public var modified: Bool
    public var pos: String
    public var branch: String?
    /// For the status bar; absent from an older core, empty lang for plain text.
    public var lang: String?
    public var indent: String?
    public var eol: String?
    public var enc: String?
}

public struct Win: Codable, Equatable, Sendable {
    public var x: Int
    public var y: Int
    public var w: Int
    public var h: Int
    public var divider: Bool
    public var active: Bool
    public var rows: [[Run]]
    public var paint: [RowPaint]?
    public var paint_gutter: Int?
    /// The selection, one span per text row; absent when there is none.
    public var sel: [Sel]?
    public var status: Status
    /// The buffer's file, absolute; nil for no-name and special buffers.
    public var path: String?
    // Phase 2, all absent from an older core.
    /// The window id, for `tab` and `scroll`.
    public var id: Int?
    /// Rows reserved at the top for the tab strip and breadcrumbs; `rows` start below them.
    public var header: Int?
    /// Columns reserved at the right for the minimap.
    public var margin: Int?
    /// The first buffer line shown, and the buffer's line count.
    public var top: Int?
    public var lines: Int?
    /// In display order, exactly one `current`.
    public var tabs: [Tab]?
    /// The last row is draw mode's toolbar, not the status bar's.
    public var toolbar: Bool?
    /// A terminal: the wheel scrolls its shell.
    public var term: Bool?
    /// The symbol the cursor is in: [name, kind].
    public var symbol: [String]?
    /// The Claude chat: drawn as VS Code's Chat panel.
    public var chat: ChatBar?
}

public struct ChatBar: Codable, Equatable, Sendable {
    public var model: String?
    public var mode: String?
    public var busy: Bool
    /// Past conversations [id, title, how long ago], while nothing is said yet.
    public var sessions: [[String]]?
}

public struct Tab: Codable, Equatable, Sendable {
    public var name: String
    public var path: String?
    public var modified: Bool
    public var preview: Bool
    public var current: Bool
    /// A directory (dired): shown with a folder icon.
    public var dir: Bool?

    public init(name: String, path: String? = nil, modified: Bool = false, preview: Bool = false, current: Bool = false) {
        self.name = name
        self.path = path
        self.modified = modified
        self.preview = preview
        self.current = current
    }
}

/// Selected cells `x..<x + w` of text row `y`, in the window.
public struct Sel: Codable, Equatable, Sendable {
    public var y: Int
    public var x: Int
    public var w: Int

    public init(y: Int, x: Int, w: Int) {
        self.y = y
        self.x = x
        self.w = w
    }
}

public struct Cursor: Codable, Equatable, Sendable {
    public var x: Int
    public var y: Int
    public var shape: String
}

public struct KeyMenu: Codable, Equatable, Sendable {
    public var x: Int
    public var y: Int
    public var w: Int
    public var rows: [[Run]]
}

public struct Frame: Codable, Sendable {
    public var seq: Int
    public var cols: Int
    public var rows: Int
    public var title: String
    public var windows: [Win]
    public var bottom: [[Run]]
    public var bottom_paint: [RowPaint]?
    public var leader: KeyMenu?
    public var cursor: Cursor?
    /// An open picker, drawn as a floating palette; absent otherwise.
    public var palette: PickerFrame?
    /// Zap's labels on the sidebar's rows (the last `nav` sent).
    public var zap_nav: [NavLabel]?
    /// Give the sidebar the keyboard (`Space w 0`, `CTRL-W h` from the leftmost window).
    public var focus_nav: Bool?
    /// Ask for a project folder (the splash's `o`), then send `cd`.
    public var open_folder: Bool?
    /// `Space w` is numbering the windows: the sidebar shows its 0.
    public var picking: Bool?
    /// The core's working directory: the project.
    public var cwd: String?
}

/// A zap label on sidebar row `row`, at character `at` of its name.
public struct NavLabel: Codable, Equatable, Sendable {
    public var row: Int
    public var at: Int
    public var typed: Int
    public var label: String
}

/// A picker: the query and rows `first..<first + rows.count` of `total`.
public struct PickerFrame: Codable, Equatable, Sendable {
    public var title: String
    public var query: String
    /// The caret, in characters.
    public var caret: Int
    /// Cmd-A selected the whole query; absent from an older core.
    public var all: Bool?
    public var rows: [PickerRow]
    public var first: Int
    public var total: Int
    /// In `rows`; nil while the typed text itself is selected.
    public var sel: Int?
    public var status: String
    public var files: Bool
}

public struct PickerRow: Codable, Equatable, Sendable {
    public var text: String
    /// Matched character ranges, [start, end).
    public var hl: [[Int]]
}

/// A window's buffer in miniature: per line, `[col, len, color]` runs
/// (color in the run encoding, nil for the default). `lines` is nil for a
/// file too big to map.
public struct Minimap: Codable, Sendable {
    public var win: Int
    public var version: Int
    public var lines: [[[Int?]]]?
}

public enum Incoming {
    case frame(Frame)
    case minimap(Minimap)
    case exit(code: Int)

    struct Tag: Decodable { var t: String }
    struct Exit: Decodable { var code: Int }

    public static func decode(_ line: Data) throws -> Incoming {
        let d = JSONDecoder()
        let tag = try d.decode(Tag.self, from: line)
        switch tag.t {
        case "frame": return .frame(try d.decode(Frame.self, from: line))
        case "minimap": return .minimap(try d.decode(Minimap.self, from: line))
        case "exit": return .exit(code: try d.decode(Exit.self, from: line).code)
        default:
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "unknown t \(tag.t)"))
        }
    }
}

public enum Outgoing {
    /// `margin` (minimap columns) is left out when 0, its default.
    case resize(cols: Int, rows: Int, margin: Int, aspect: Int = 0)
    case keys(seq: Int, keys: String)
    case paste(seq: Int, text: String)
    case mouse(seq: Int, kind: String, col: Int, row: Int)
    case mac(seq: Int, key: String, shift: Bool)
    /// `preview` is left out when false, its default.
    case open(seq: Int, path: String, preview: Bool)
    /// A file dropped on window `target`'s `zone` (center or an edge).
    case dropFile(seq: Int, path: String, target: Int, zone: String)
    /// `op`: select, close, move (to `to`) or keep.
    case tab(seq: Int, win: Int, op: String, index: Int, to: Int?)
    /// A tab dragged out of window `win` onto window `target`'s `zone`.
    case drop(seq: Int, win: Int, index: Int, target: Int, zone: String, to: Int)
    case scroll(seq: Int, win: Int, top: Int)
    /// A click on picker row `index` (absolute).
    case pick(seq: Int, index: Int)
    /// The terminal panel: `op` toggle or maximize.
    case panel(seq: Int, op: String)
    /// Work in folder `path` (`:cd`): the chosen project.
    case cd(seq: Int, path: String)
    /// The File menu: `op` new, save_as (to `path`), save_all or revert.
    case file(seq: Int, op: String, path: String?)
    /// The chat panel: `op` toggle, new, resume (conversation `arg`), model or mode.
    case chat(seq: Int, op: String, arg: String?)
    /// The Edit and Selection menus' editing commands.
    case menu(seq: Int, op: String)
    /// The sidebar's visible rows (name, path), for zap.
    case nav(rows: [(name: String, path: String)])
    case quit

    /// One JSON object plus "\n", fields in a fixed order.
    public func line() -> Data {
        func q(_ s: String) -> String {
            let d = try! JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed, .withoutEscapingSlashes])
            return String(decoding: d, as: UTF8.self)
        }
        let body: String
        switch self {
        case .resize(let c, let r, let m, let a): body = #""resize","cols":\#(c),"rows":\#(r)"# + (m == 0 ? "" : #","margin":\#(m)"#) + (a == 0 ? "" : #","aspect":\#(a)"#)
        case .keys(let s, let k): body = #""keys","seq":\#(s),"keys":\#(q(k))"#
        case .paste(let s, let t): body = #""paste","seq":\#(s),"text":\#(q(t))"#
        case .mouse(let s, let k, let c, let r): body = #""mouse","seq":\#(s),"kind":\#(q(k)),"col":\#(c),"row":\#(r)"#
        case .mac(let s, let k, let sh): body = #""mac","seq":\#(s),"key":\#(q(k)),"shift":\#(sh)"#
        case .open(let s, let p, let pv): body = #""open","seq":\#(s),"path":\#(q(p))"# + (pv ? #","preview":true"# : "")
        case .dropFile(let s, let p, let tg, let z):
            body = #""open","seq":\#(s),"path":\#(q(p)),"target":\#(tg),"zone":\#(q(z))"#
        case .tab(let s, let w, let op, let i, let to):
            body = #""tab","seq":\#(s),"win":\#(w),"op":\#(q(op)),"index":\#(i)"# + (to.map { #","to":\#($0)"# } ?? "")
        case .drop(let s, let w, let i, let tg, let z, let to):
            body = #""tab","seq":\#(s),"win":\#(w),"op":"drop","index":\#(i),"to":\#(to),"target":\#(tg),"zone":\#(q(z))"#
        case .scroll(let s, let w, let t): body = #""scroll","seq":\#(s),"win":\#(w),"top":\#(t)"#
        case .pick(let s, let i): body = #""pick","seq":\#(s),"index":\#(i)"#
        case .panel(let s, let op): body = #""panel","seq":\#(s),"op":\#(q(op))"#
        case .file(let s, let op, let p): body = #""file","seq":\#(s),"op":\#(q(op))"# + (p.map { #","path":\#(q($0))"# } ?? "")
        case .menu(let s, let op): body = #""menu","seq":\#(s),"op":\#(q(op))"#
        case .chat(let s, let op, let a): body = #""chat","seq":\#(s),"op":\#(q(op))"# + (a.map { #","arg":\#(q($0))"# } ?? "")
        case .cd(let s, let p): body = #""cd","seq":\#(s),"path":\#(q(p))"#
        case .nav(let rows):
            body = #""nav","rows":["# + rows.map { #"{"name":\#(q($0.name)),"path":\#(q($0.path))}"# }.joined(separator: ",") + "]"
        case .quit: body = #""quit""#
        }
        return Data((#"{"t":"# + body + "}\n").utf8)
    }
}
