import Foundation

public struct Run: Codable, Equatable, Sendable {
    public var x: Int
    public var s: String
    public var fg: Int?
    public var bg: Int?
    public var f: Int?
}

public struct Status: Codable, Equatable, Sendable {
    public var mode: String
    public var name: String
    public var modified: Bool
    public var pos: String
    public var branch: String?
}

public struct Win: Codable, Equatable, Sendable {
    public var x: Int
    public var y: Int
    public var w: Int
    public var h: Int
    public var divider: Bool
    public var active: Bool
    public var rows: [[Run]]
    /// The selection, one span per text row; absent when there is none.
    public var sel: [Sel]?
    public var status: Status
    /// The buffer's file, absolute; nil for no-name and special buffers.
    public var path: String?
    // Phase 2, all absent from an older core.
    /// The window id, for `tab` and `scroll`.
    public var id: Int?
    /// Rows reserved at the top for the tab strip (0 or 1); `rows` start below them.
    public var header: Int?
    /// Columns reserved at the right for the minimap.
    public var margin: Int?
    /// The first buffer line shown, and the buffer's line count.
    public var top: Int?
    public var lines: Int?
    /// In display order, exactly one `current`.
    public var tabs: [Tab]?
}

public struct Tab: Codable, Equatable, Sendable {
    public var name: String
    public var path: String?
    public var modified: Bool
    public var preview: Bool
    public var current: Bool

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

public struct Frame: Codable, Sendable {
    public var seq: Int
    public var cols: Int
    public var rows: Int
    public var title: String
    public var windows: [Win]
    public var bottom: [[Run]]
    public var cursor: Cursor?
}

public enum Incoming {
    case frame(Frame)
    case exit(code: Int)

    struct Tag: Decodable { var t: String }
    struct Exit: Decodable { var code: Int }

    public static func decode(_ line: Data) throws -> Incoming {
        let d = JSONDecoder()
        let tag = try d.decode(Tag.self, from: line)
        switch tag.t {
        case "frame": return .frame(try d.decode(Frame.self, from: line))
        case "exit": return .exit(code: try d.decode(Exit.self, from: line).code)
        default:
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "unknown t \(tag.t)"))
        }
    }
}

public enum Outgoing {
    /// `margin` (minimap columns) is left out when 0, its default.
    case resize(cols: Int, rows: Int, margin: Int)
    case keys(seq: Int, keys: String)
    case paste(seq: Int, text: String)
    case mouse(seq: Int, kind: String, col: Int, row: Int)
    case mac(seq: Int, key: String, shift: Bool)
    /// `preview` is left out when false, its default.
    case open(seq: Int, path: String, preview: Bool)
    /// `op`: select, close, move (to `to`) or keep.
    case tab(seq: Int, win: Int, op: String, index: Int, to: Int?)
    case scroll(seq: Int, win: Int, top: Int)
    case quit

    /// One JSON object plus "\n", fields in a fixed order.
    public func line() -> Data {
        func q(_ s: String) -> String {
            let d = try! JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed, .withoutEscapingSlashes])
            return String(decoding: d, as: UTF8.self)
        }
        let body: String
        switch self {
        case .resize(let c, let r, let m): body = #""resize","cols":\#(c),"rows":\#(r)"# + (m == 0 ? "" : #","margin":\#(m)"#)
        case .keys(let s, let k): body = #""keys","seq":\#(s),"keys":\#(q(k))"#
        case .paste(let s, let t): body = #""paste","seq":\#(s),"text":\#(q(t))"#
        case .mouse(let s, let k, let c, let r): body = #""mouse","seq":\#(s),"kind":\#(q(k)),"col":\#(c),"row":\#(r)"#
        case .mac(let s, let k, let sh): body = #""mac","seq":\#(s),"key":\#(q(k)),"shift":\#(sh)"#
        case .open(let s, let p, let pv): body = #""open","seq":\#(s),"path":\#(q(p))"# + (pv ? #","preview":true"# : "")
        case .tab(let s, let w, let op, let i, let to):
            body = #""tab","seq":\#(s),"win":\#(w),"op":\#(q(op)),"index":\#(i)"# + (to.map { #","to":\#($0)"# } ?? "")
        case .scroll(let s, let w, let t): body = #""scroll","seq":\#(s),"win":\#(w),"top":\#(t)"#
        case .quit: body = #""quit""#
        }
        return Data((#"{"t":"# + body + "}\n").utf8)
    }
}
