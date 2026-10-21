import AppKit
import FredKit

var failures = 0
var count = 0
@MainActor func check(_ name: String, _ cond: Bool) {
    count += 1
    if cond { print("ok \(count)") } else { print("FAIL \(name)"); failures += 1 }
}

let line = #"{"t":"frame","seq":3,"cols":20,"rows":6,"title":"t","windows":[{"x":0,"y":0,"w":20,"h":5,"divider":false,"active":true,"rows":[[{"x":0,"s":"hello","fg":33515186,"f":1}]],"status":{"mode":"NORMAL","name":"f","modified":false,"pos":"1:1","branch":null}}],"bottom":[],"cursor":{"x":0,"y":0,"shape":"block"}}"#
if case .frame(let f) = try Incoming.decode(Data(line.utf8)) {
    check("seq", f.seq == 3)
    check("run text", f.windows[0].rows[0][0].s == "hello")
    check("bold", f.windows[0].rows[0][0].f == 1)
} else { check("decodes a frame", false) }
check("exit", { if case .exit(let c) = try! Incoming.decode(Data(#"{"t":"exit","code":0}"#.utf8)) { return c == 0 }; return false }())
check("keys line", String(decoding: Outgoing.keys(seq: 7, keys: "<C-w>j").line(), as: UTF8.self) == #"{"t":"keys","seq":7,"keys":"<C-w>j"}"# + "\n")
let p = Palette(fg: .white, bg: .black)
check("rgb", p.color(0x1ff66b2, default: .white).usingColorSpace(.sRGB)!.redComponent == 1.0)
check("default", p.color(nil, default: .black) == .black)
func hex(_ c: NSColor) -> Int {
    let s = c.usingColorSpace(.sRGB)!
    return Int((s.redComponent * 255).rounded()) << 16 | Int((s.greenComponent * 255).rounded()) << 8 | Int((s.blueComponent * 255).rounded())
}
let md = Palette.materialDark
check("material dark bg", hex(md.bg) == 0x212121)
check("material dark fg", hex(md.fg) == 0xeeffff)
check("material dark ansi", (0..<16).map { hex(md.color($0, default: .clear)) } == [
    0x000000, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xeeffff,
    0x545454, 0xff5370, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xffffff,
])
check("snap 2x", snap(10.3, scale: 2) == 10.5)
check("snap 1x", snap(10.3, scale: 1) == 10.0)
let c = cellSize(font: .monospacedSystemFont(ofSize: 13, weight: .regular), scale: 2)
check("cell height on pixel grid", (c.height * 2).rounded() == c.height * 2)
check("cell width positive", c.width > 7 && c.width < 9)
check("wide chars take 2 cells", cellWidth("日本語 wide") == 11)
check("ascii 1 cell", cellWidth("abc") == 3)
var acc: CGFloat = 0
check("wheel notch is one line", scrollLines(-0.1, precise: false, acc: &acc) == -1 && scrollLines(5, precise: false, acc: &acc) == 1)
check("trackpad adds up 3 per line", scrollLines(2, precise: true, acc: &acc) == 0 && scrollLines(2, precise: true, acc: &acc) == 1 && acc == 1)
check("trackpad turning starts over", scrollLines(-2, precise: true, acc: &acc) == 0 && acc == -2)

func n(_ c: String, _ ign: String? = nil, code: UInt16 = 0, ctrl: Bool = false, opt: Bool = false, shift: Bool = false, cmd: Bool = false) -> String? {
    notation(chars: c, charsIgnoringModifiers: ign ?? c, keyCode: code, control: ctrl, option: opt, shift: shift, command: cmd)
}
check("plain", n("a") == "a")
check("lt", n("<") == "<lt>")
check("ctrl", n("\u{17}", "w", ctrl: true) == "<C-w>")
check("ctrl lowercases", n("\u{17}", "W", ctrl: true) == "<C-w>")
check("option is alt", n("∆", "j", opt: true) == "<M-j>")
check("ctrl alt", n("", "x", ctrl: true, opt: true) == "<C-M-x>")
check("esc", n("\u{1b}", code: 53) == "<Esc>")
check("enter", n("\r", code: 36) == "<Enter>" && n("\r", code: 76) == "<Enter>")
check("bs del", n("\u{7f}", code: 51) == "<BS>" && n("", code: 117) == "<Del>")
check("shift tab", n("\u{19}", code: 48, shift: true) == "<S-Tab>")
check("tab", n("\t", code: 48) == "<Tab>")
check("shift left", n("", code: 123, shift: true) == "<S-Left>")
check("arrows", [124, 125, 126].map { n("", code: $0) } == ["<Right>", "<Down>", "<Up>"])
check("nav", [115, 119, 116, 121].map { n("", code: $0) } == ["<Home>", "<End>", "<PageUp>", "<PageDown>"])
check("cmd goes to menus", n("s", cmd: true) == nil)
check("space", n(" ") == " ")
check("function key is dropped", n("\u{f704}", code: 122) == nil)
check("option shift keeps case", n("Ï", "J", opt: true, shift: true) == "<M-J>")
check("gt under option/ctrl", n("≥", ">", opt: true) == "<M-gt>" && n("", ">", ctrl: true) == "<C-gt>" && n("", ">", ctrl: true, opt: true) == "<C-M-gt>")
check("ctrl function key dropped", n("", "\u{f704}", code: 122, ctrl: true) == nil && n("", "\u{f704}", code: 122, opt: true) == nil)

// Selection shapes: rows that touch join into one outline; corners round.
let frameWithSel = #"{"t":"frame","seq":1,"cols":20,"rows":6,"title":"t","windows":[{"x":0,"y":0,"w":20,"h":5,"divider":false,"active":true,"rows":[],"sel":[{"y":0,"x":2,"w":4}],"status":{"mode":"SELECT","name":"f","modified":false,"pos":"1:1","branch":null}}],"bottom":[],"cursor":null}"#
if case .frame(let f) = try Incoming.decode(Data(frameWithSel.utf8)) {
    check("sel decodes", f.windows[0].sel == [Sel(y: 0, x: 2, w: 4)])
} else { check("decodes a frame with sel", false) }
if case .frame(let f) = try Incoming.decode(Data(line.utf8)) { check("no sel is nil", f.windows[0].sel == nil) }
let cell = CGSize(width: 10, height: 20)
func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x, y: y) }
check("one row is a box", selectionOutlines([Sel(y: 1, x: 2, w: 3)], cell: cell, scale: 2)
    == [[pt(20, 20), pt(50, 20), pt(50, 40), pt(20, 40)]])
// abc|      row 0: cols 2..<6
// defghi    row 1: cols 0..<6  -> one outline, 6 corners
// de        row 2: cols 0..<2
let staircase = [Sel(y: 0, x: 2, w: 4), Sel(y: 1, x: 0, w: 6), Sel(y: 2, x: 0, w: 2)]
let o = selectionOutlines(staircase, cell: cell, scale: 2)
check("touching rows make one outline", o.count == 1)
check("its corners", o.first == [pt(20, 0), pt(60, 0), pt(60, 40), pt(20, 40), pt(20, 60), pt(0, 60), pt(0, 20), pt(20, 20)]
    .rotatedToStart(at: o.first?.first))
check("rows that don't overlap stay apart", selectionOutlines([Sel(y: 0, x: 4, w: 2), Sel(y: 1, x: 0, w: 4)], cell: cell, scale: 2).count == 2)
check("rows with a gap stay apart", selectionOutlines([Sel(y: 0, x: 0, w: 2), Sel(y: 2, x: 0, w: 2)], cell: cell, scale: 2).count == 2)
check("empty spans draw nothing", selectionOutlines([Sel(y: 0, x: 0, w: 0)], cell: cell, scale: 2).isEmpty)
let path = selectionPath(staircase, cell: cell, scale: 2)
func near(_ a: CGRect, _ b: CGRect) -> Bool {
    abs(a.minX - b.minX) < 1e-6 && abs(a.minY - b.minY) < 1e-6 && abs(a.maxX - b.maxX) < 1e-6 && abs(a.maxY - b.maxY) < 1e-6
}
check("path covers the rows", near(path.boundingBoxOfPath, CGRect(x: 0, y: 0, width: 60, height: 60)))
check("inside is filled", path.contains(pt(30, 30)))
check("outer corner is rounded off", !path.contains(pt(60.5 - 1, 0.5)) && path.contains(pt(55, 5)))
// Inside corner where row 0 (from col 2) meets row 1 (from col 0): filled out a little.
check("inside corner curves out", path.contains(pt(19, 19)) && !path.contains(pt(10, 10)))
check("one-cell box keeps its size", near(selectionPath([Sel(y: 0, x: 0, w: 1)], cell: cell, scale: 2).boundingBoxOfPath,
                                         CGRect(x: 0, y: 0, width: 10, height: 20)))

var l = Latency()
l.start(seq: 1, at: 0.000); l.start(seq: 2, at: 0.001)
l.shown(seq: 2, at: 0.006)
check("closes both", l.count == 2)
check("p50", abs(l.p50 - 5.0) < 0.01 || abs(l.p50 - 6.0) < 0.01)
check("p99 is the max of two", abs(l.p99 - 6.0) < 0.01)
l.shown(seq: 2, at: 0.9)
check("repeat seq adds nothing", l.count == 2)

check("open line", String(decoding: Outgoing.open(seq: 4, path: "/a/b c", preview: false).line(), as: UTF8.self) == #"{"t":"open","seq":4,"path":"/a/b c"}"# + "\n")
let withPath = line.replacingOccurrences(of: #""active":true,"#, with: #""active":true,"path":"/r/f","#)
if case .frame(let f) = try Incoming.decode(Data(withPath.utf8)) { check("win path", f.windows[0].path == "/r/f") } else { check("win path", false) }
if case .frame(let f) = try Incoming.decode(Data(line.utf8)) { check("no win path", f.windows[0].path == nil) } else { check("no win path", false) }
let hid = { (n: String, d: Bool, c: Bool) in FileTree.isHidden(n, isDir: d, cargo: c) }
check("hidden", hid(".git", true, false) && hid(".git", false, false) && hid(".DS_Store", false, false))
check("dotfiles shown", !hid(".gitignore", false, false) && !hid(".github", true, false) && !hid("node_modules", true, false))
check("target only beside Cargo.toml", hid("target", true, true) && !hid("target", true, false) && !hid("target", false, true))
check("canonical resolves /tmp", FileTree.canonical("/tmp") == "/private/tmp" && FileTree.canonical("/var/../tmp") == "/private/tmp")
check("canonical keeps a missing path", FileTree.canonical("/no/such/fred") == "/no/such/fred")
check("folders first, case-insensitive", FileTree.sorted([("b.rs", false), ("Zed", true), ("A.md", false), ("apps", true), (".github", true)]).map(\.name)
      == [".github", "apps", "Zed", "A.md", "b.rs"])
check("ancestors", FileTree.ancestors(of: "/r/src/gui/mod.rs", under: "/r") == ["/r/src", "/r/src/gui"])
check("ancestors top level", FileTree.ancestors(of: "/r/a.rs", under: "/r/") == [])
check("ancestors outside", FileTree.ancestors(of: "/rx/a.rs", under: "/r") == nil && FileTree.ancestors(of: "/r", under: "/r") == nil)
check("symbols", FileTree.symbol(for: "x", isDir: true) == "folder" && FileTree.symbol(for: "a.RS", isDir: false) == "chevron.left.forwardslash.chevron.right"
      && FileTree.symbol(for: "README.md", isDir: false) == "doc.text" && FileTree.symbol(for: "a.bin", isDir: false) == "doc")

// Tab strip layout.
do {
    let fit = TabMath.layout(widths: [100, 150, 120], available: 1000, current: 1)
    check("tabs that fit keep natural widths", fit.frames.map(\.width) == [100, 150, 120] && fit.offset == 0)
    check("tabs sit side by side", fit.frames.map(\.minX) == [0, 100, 250])
    check("widths clamp to 90...220", TabMath.layout(widths: [40, 400], available: 1000, current: 0).frames.map(\.width) == [90, 220])
    let tight = TabMath.layout(widths: [100, 220, 220], available: 420, current: 2)
    check("overflow shrinks to fill", abs(tight.frames.map(\.width).reduce(0, +) - 420) < 0.001 && tight.offset == 0)
    check("overflow shrinks every tab to at least 90", tight.frames.allSatisfy { $0.width >= 90 })
    check("the widest shrink first (evenly)", tight.frames[0].width == 100 && tight.frames[1].width == tight.frames[2].width)
    let many = TabMath.layout(widths: Array(repeating: 200, count: 10), available: 450, current: 7)
    check("beyond that tabs stop at 90", many.frames.allSatisfy { $0.width == 90 })
    let cur = many.frames[7]
    check("and scroll so the current tab is fully visible", cur.minX - many.offset >= 0 && cur.maxX - many.offset <= 450 && many.offset > 0)
    check("first tab current needs no scroll", TabMath.layout(widths: Array(repeating: 200, count: 10), available: 450, current: 0).offset == 0)
    check("a kept offset stays while current is visible",
          TabMath.layout(widths: Array(repeating: 200, count: 10), available: 450, current: 6, offset: many.offset).offset == many.offset)
    check("offset never runs past the end", TabMath.layout(widths: Array(repeating: 200, count: 10), available: 450, current: 9, offset: 5000).offset == 900 - 450)
    check("no tabs", TabMath.layout(widths: [], available: 450, current: 0).frames.isEmpty)
}

// Phase 2 window fields: present, and absent (an older core).
let withTabs = line.replacingOccurrences(of: #""active":true,"#, with: #""active":true,"id":3,"header":1,"margin":8,"top":120,"lines":2048,"tabs":[{"name":"main.rs","path":"/abs/src/main.rs","modified":false,"preview":true,"current":true},{"name":"[No Name]","modified":true,"preview":false,"current":false}],"#)
if case .frame(let f) = try Incoming.decode(Data(withTabs.utf8)) {
    let w = f.windows[0]
    check("win phase 2 fields", w.id == 3 && w.header == 1 && w.margin == 8 && w.top == 120 && w.lines == 2048)
    check("tabs decode", w.tabs?.count == 2 && w.tabs?[0].preview == true && w.tabs?[0].path == "/abs/src/main.rs" && w.tabs?[1].path == nil && w.tabs?[1].modified == true)
} else { check("decodes a frame with tabs", false) }
if case .frame(let f) = try Incoming.decode(Data(line.utf8)) {
    let w = f.windows[0]
    check("old frame: no phase 2 fields", w.id == nil && w.tabs == nil && w.header == nil && w.margin == nil)
} else { check("old frame decodes", false) }
func out(_ o: Outgoing) -> String { String(decoding: o.line(), as: UTF8.self) }
check("tab line", out(.tab(seq: 6, win: 3, op: "move", index: 2, to: 0)) == #"{"t":"tab","seq":6,"win":3,"op":"move","index":2,"to":0}"# + "\n")
check("tab line without to", out(.tab(seq: 6, win: 3, op: "select", index: 2, to: nil)) == #"{"t":"tab","seq":6,"win":3,"op":"select","index":2}"# + "\n")
check("scroll line", out(.scroll(seq: 10, win: 3, top: 400)) == #"{"t":"scroll","seq":10,"win":3,"top":400}"# + "\n")
check("open preview line", out(.open(seq: 5, path: "/f", preview: true)) == #"{"t":"open","seq":5,"path":"/f","preview":true}"# + "\n")
check("open line omits preview false", out(.open(seq: 5, path: "/f", preview: false)) == #"{"t":"open","seq":5,"path":"/f"}"# + "\n")
check("resize margin", out(.resize(cols: 120, rows: 40, margin: 8)) == #"{"t":"resize","cols":120,"rows":40,"margin":8}"# + "\n")
check("resize margin 0 omitted", out(.resize(cols: 120, rows: 40, margin: 0)) == #"{"t":"resize","cols":120,"rows":40}"# + "\n")

// Minimap: 2 pt per line; scrolls only when taller than the view.
do {
    check("short file doesn't scroll", MinimapMath.offset(top: 10, lines: 100, visible: 30, height: 400) == 0)
    // 1000 lines = 2000 pt in 400 pt: at the end (top 970) the offset is 1600.
    check("long file at the start", MinimapMath.offset(top: 0, lines: 1000, visible: 30, height: 400) == 0)
    check("long file at the end", MinimapMath.offset(top: 970, lines: 1000, visible: 30, height: 400) == 1600)
    check("long file halfway", MinimapMath.offset(top: 485, lines: 1000, visible: 30, height: 400) == 800)
    check("past the end clamps", MinimapMath.offset(top: 999, lines: 1000, visible: 30, height: 400) == 1600)
    check("line at y", MinimapMath.line(atY: 0, offset: 0) == 0 && MinimapMath.line(atY: 5, offset: 0) == 2
          && MinimapMath.line(atY: 5, offset: 800) == 402)
    check("centering", MinimapMath.topForCentering(line: 500, visible: 30, lines: 1000) == 485)
    check("centering clamps low", MinimapMath.topForCentering(line: 3, visible: 30, lines: 1000) == 0)
    check("centering clamps high", MinimapMath.topForCentering(line: 995, visible: 30, lines: 1000) == 970)
    check("centering a short file", MinimapMath.topForCentering(line: 10, visible: 30, lines: 20) == 0)
    // Dragging: the box's y and back.
    for (top, lines) in [(0, 1000), (300, 1000), (970, 1000), (40, 100)] {
        let y = MinimapMath.boxY(top: top, lines: lines, visible: 30, height: 400)
        check("drag round trip \(top)/\(lines)", MinimapMath.top(forBoxY: y, lines: lines, visible: 30, height: 400) == top)
    }
    check("drag clamps", MinimapMath.top(forBoxY: -50, lines: 1000, visible: 30, height: 400) == 0
          && MinimapMath.top(forBoxY: 5000, lines: 1000, visible: 30, height: 400) == 970)
}
let mm = #"{"t":"minimap","win":3,"version":17,"lines":[[[0,4,16737970],[5,3,null]],[]]}"#
if case .minimap(let m) = try Incoming.decode(Data(mm.utf8)) {
    check("minimap decodes", m.win == 3 && m.version == 17 && m.lines?.count == 2 && m.lines?[1].isEmpty == true)
    check("minimap runs", m.lines?[0] == [[0, 4, 16737970], [5, 3, nil]])
} else { check("decodes a minimap", false) }
if case .minimap(let m) = try Incoming.decode(Data(#"{"t":"minimap","win":3,"version":18,"lines":null}"#.utf8)) {
    check("huge file: null lines", m.lines == nil)
} else { check("decodes a null minimap", false) }
check("unknown message throws, doesn't crash", (try? Incoming.decode(Data(#"{"t":"later","x":1}"#.utf8))) == nil)

// Compact runs: what's kept per window instead of the decoded JSON.
check("compact runs", MinimapMath.compact([[[0, 4, 16737970], [5, 3, nil]], [], [[2, 1]]])
      == [MiniRun(line: 0, col: 0, len: 4, color: 16737970), MiniRun(line: 0, col: 5, len: 3, color: nil),
          MiniRun(line: 2, col: 2, len: 1, color: nil)])
check("compact clips at 96 columns, drops junk", MinimapMath.compact([[[94, 10, 1], [96, 2, 1], [3, 0, 1], [nil, 2, 1], [4]]])
      == [MiniRun(line: 0, col: 94, len: 2, color: 1)])

exit(failures == 0 ? 0 : 1)

extension Array where Element == CGPoint {
    /// The same cycle of points, starting at `p`.
    func rotatedToStart(at p: CGPoint?) -> [CGPoint] {
        guard let p, let i = firstIndex(of: p) else { return self }
        return Array(self[i...] + self[..<i])
    }
}
