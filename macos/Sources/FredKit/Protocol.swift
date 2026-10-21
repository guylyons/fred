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
    case resize(cols: Int, rows: Int)
    case keys(seq: Int, keys: String)
    case paste(seq: Int, text: String)
    case mouse(seq: Int, kind: String, col: Int, row: Int)
    case mac(seq: Int, key: String, shift: Bool)
    case quit

    /// One JSON object plus "\n", fields in a fixed order.
    public func line() -> Data {
        func q(_ s: String) -> String {
            let d = try! JSONSerialization.data(withJSONObject: s, options: .fragmentsAllowed)
            return String(decoding: d, as: UTF8.self)
        }
        let body: String
        switch self {
        case .resize(let c, let r): body = #""resize","cols":\#(c),"rows":\#(r)"#
        case .keys(let s, let k): body = #""keys","seq":\#(s),"keys":\#(q(k))"#
        case .paste(let s, let t): body = #""paste","seq":\#(s),"text":\#(q(t))"#
        case .mouse(let s, let k, let c, let r): body = #""mouse","seq":\#(s),"kind":\#(q(k)),"col":\#(c),"row":\#(r)"#
        case .mac(let s, let k, let sh): body = #""mac","seq":\#(s),"key":\#(q(k)),"shift":\#(sh)"#
        case .quit: body = #""quit""#
        }
        return Data((#"{"t":"# + body + "}\n").utf8)
    }
}
