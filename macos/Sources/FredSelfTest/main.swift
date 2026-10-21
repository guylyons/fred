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
check("snap 2x", snap(10.3, scale: 2) == 10.5)
check("snap 1x", snap(10.3, scale: 1) == 10.0)
let c = cellSize(font: .monospacedSystemFont(ofSize: 13, weight: .regular), scale: 2)
check("cell height on pixel grid", (c.height * 2).rounded() == c.height * 2)
check("cell width positive", c.width > 7 && c.width < 9)
check("wide chars take 2 cells", cellWidth("日本語 wide") == 11)
check("ascii 1 cell", cellWidth("abc") == 3)

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

exit(failures == 0 ? 0 : 1)
