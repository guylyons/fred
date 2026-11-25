import AppKit
import SwiftUI

/// Fred ▸ Settings…: the theme, background and editor font, live. The app
/// owns the values; it fills this in (`quiet`, so nothing echoes back) and
/// hears each change through `hooks`.
@MainActor
final class SettingsModel: ObservableObject {
    struct Hooks {
        var glass: (Bool) -> Void = { _ in }
        var minimap: (Bool) -> Void = { _ in }
        /// A UserDefaults key and its value; `done` when the slider is let go.
        var background: (String, Double, _ done: Bool) -> Void = { _, _, _ in }
        var chooseImage: () -> Void = {}
        var removeImage: () -> Void = {}
        var font: (String, CGFloat) -> Void = { _, _ in }
        var moreFonts: () -> Void = {}
    }

    var hooks = Hooks()
    private var quiet = false

    @Published var glass = false { didSet { if !quiet, glass != oldValue { hooks.glass(glass) } } }
    @Published var minimap = false { didSet { if !quiet, minimap != oldValue { hooks.minimap(minimap) } } }
    @Published var image: NSImage?
    @Published var opacity = 0.75
    @Published var blur = 0.0
    @Published var brightness = 0.0
    @Published var family = "" {
        didSet {
            guard !quiet, family != oldValue else { return }
            styles = SettingsModel.styles(of: family)
            fontName = (styles.first { $0.style == "Regular" } ?? styles.first)?.name ?? fontName
        }
    }
    @Published var fontName = "" { didSet { if !quiet, fontName != oldValue { hooks.font(fontName, fontSize) } } }
    @Published var fontSize = 15.0 { didSet { if !quiet, fontSize != oldValue { hooks.font(fontName, fontSize) } } }
    @Published var styles: [Style] = []

    struct Style: Hashable { var name: String; var style: String }

    /// Fixed-pitch families, for the editor.
    let families: [String] = {
        let names = NSFontManager.shared.availableFontNames(with: .fixedPitchFontMask) ?? []
        return Set(names.compactMap { NSFont(name: $0, size: 12)?.familyName }).sorted()
    }()

    static func styles(of family: String) -> [Style] {
        (NSFontManager.shared.availableMembers(ofFontFamily: family) ?? []).compactMap { m in
            guard let name = m.first as? String, let style = m.dropFirst().first as? String else { return nil }
            return Style(name: name, style: style)
        }
    }

    /// The app's current values, without calling back.
    func sync(glass: Bool, minimap: Bool, image: NSImage?, font: NSFont) {
        quiet = true
        defer { quiet = false }
        let d = UserDefaults.standard
        self.glass = glass
        self.minimap = minimap
        self.image = image
        opacity = d.double(forKey: "FredOpacity")
        blur = d.double(forKey: "FredImageBlur")
        brightness = d.double(forKey: "FredImageBrightness")
        family = font.familyName ?? ""
        styles = SettingsModel.styles(of: family)
        fontName = font.fontName
        fontSize = font.pointSize
    }
}

struct SettingsView: View {
    @ObservedObject var m: SettingsModel

    var body: some View {
        TabView {
            appearance.tabItem { Label("Appearance", systemImage: "paintpalette") }
            editor.tabItem { Label("Editor", systemImage: "textformat") }
        }
        .padding(.top, 8)
        .frame(width: 540, height: 560)
    }

    private var appearance: some View {
        Form {
            Section("Theme") {
                Picker("Theme", selection: $m.glass) {
                    Text("Deep Ocean").tag(false)
                    Text("Glass").tag(true)
                }
                .pickerStyle(.segmented)
                Toggle("Show minimap", isOn: $m.minimap)
            }

            Section {
                HStack(spacing: 16) {
                    thumbnail
                    VStack(alignment: .leading, spacing: 8) {
                        Button("Choose Image…") { m.hooks.chooseImage() }
                        Button("Remove", role: .destructive) { m.hooks.removeImage() }
                            .disabled(m.image == nil)
                    }
                    Spacer()
                }
                .padding(.vertical, 4)
                slider("Opacity", $m.opacity, 0...1, key: "FredOpacity") { "\(Int(($0 * 100).rounded()))%" }
                    .disabled(!m.glass && m.image == nil)
                slider("Blur", $m.blur, 0...60, key: "FredImageBlur") { "\(Int($0.rounded()))" }
                    .disabled(m.image == nil)
                slider("Brightness", $m.brightness, -0.5...0.5, key: "FredImageBrightness") { String(format: "%+.0f%%", $0 * 100) }
                    .disabled(m.image == nil)
            } header: {
                Text("Background")
            } footer: {
                Text("The picture sits behind the panes. Opacity is how much of the theme covers it, or the desktop in Glass.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var thumbnail: some View {
        RoundedRectangle(cornerRadius: 8)
            .fill(Color(nsColor: .quaternaryLabelColor))
            .overlay {
                if let image = m.image {
                    Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
                } else {
                    Image(systemName: "photo").font(.title2).foregroundStyle(.secondary)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
            .frame(width: 128, height: 80)
    }

    private func slider(_ label: String, _ value: Binding<Double>, _ range: ClosedRange<Double>, key: String,
                        _ text: @escaping (Double) -> String) -> some View {
        LabeledContent(label) {
            HStack {
                Slider(value: value, in: range) { editing in
                    if !editing { m.hooks.background(key, value.wrappedValue, true) }
                }
                .onChange(of: value.wrappedValue) { _, v in m.hooks.background(key, v, false) }
                Text(text(value.wrappedValue)).monospacedDigit().foregroundStyle(.secondary).frame(width: 48, alignment: .trailing)
            }
        }
    }

    private var editor: some View {
        Form {
            Section {
                Picker("Family", selection: $m.family) {
                    ForEach(m.families.contains(m.family) ? m.families : [m.family] + m.families, id: \.self) { Text($0).tag($0) }
                }
                Picker("Style", selection: $m.fontName) {
                    ForEach(m.styles, id: \.name) { Text($0.style).tag($0.name) }
                }
                LabeledContent("Size") {
                    HStack {
                        Slider(value: $m.fontSize, in: 9...36, step: 1)
                        Stepper("", value: $m.fontSize, in: 6...72, step: 1).labelsHidden()
                        Text("\(Int(m.fontSize)) pt").monospacedDigit().foregroundStyle(.secondary).frame(width: 48, alignment: .trailing)
                    }
                }
                HStack {
                    Spacer()
                    Button("More Fonts…") { m.hooks.moreFonts() }
                }
            } header: {
                Text("Font")
            } footer: {
                Text("Saved to ~/.config/fred/config.toml as font and font_size.")
                    .font(.footnote).foregroundStyle(.secondary)
            }

            Section("Preview") {
                Text("fn main() {\n    let fred = \"ed, grown up\"; // 0O 1lI {}[] => !=\n    println!(\"{fred}\");\n}")
                    .font(.custom(m.fontName, size: m.fontSize))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Color(red: 0.06, green: 0.08, blue: 0.13)))
                    .foregroundStyle(Color(red: 0.79, green: 0.81, blue: 0.86))
            }
        }
        .formStyle(.grouped)
    }
}
