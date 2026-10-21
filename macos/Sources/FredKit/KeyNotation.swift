/// Fred's vim notation for a key event; nil for Cmd chords (the menu bar's) and keys with no text.
public func notation(chars: String, charsIgnoringModifiers: String, keyCode: UInt16,
                     control: Bool, option: Bool, shift: Bool, command: Bool) -> String? {
    if command { return nil }
    let special: [UInt16: String] = [
        53: "Esc", 36: "Enter", 76: "Enter", 51: "BS", 48: "Tab", 117: "Del",
        123: "Left", 124: "Right", 125: "Down", 126: "Up",
        115: "Home", 119: "End", 116: "PageUp", 121: "PageDown",
    ]
    if let name = special[keyCode] {
        let shiftable: Set<UInt16> = [48, 123, 124, 125, 126]
        return "<" + (shift && shiftable.contains(keyCode) ? "S-" : "") + name + ">"
    }
    // F-keys and friends arrive as private-use characters.
    func isFunctionKey(_ s: String) -> Bool { s.unicodeScalars.first.map { (0xF700...0xF8FF).contains($0.value) } ?? false }
    if control || option {
        // Control folds case (as Rust does); Option keeps Shift.
        let k = control ? charsIgnoringModifiers.lowercased() : charsIgnoringModifiers
        guard !k.isEmpty, !isFunctionKey(k) else { return nil }
        let name = k == "<" ? "lt" : k == ">" ? "gt" : k
        return "<" + (control ? "C-" : "") + (option ? "M-" : "") + name + ">"
    }
    guard !chars.isEmpty, !isFunctionKey(chars) else { return nil }
    return chars == "<" ? "<lt>" : chars
}
