import SwiftUI

/// One bia this phone holds the key of. `host` is what is dialled, a port included.
struct ChatTarget: Identifiable {
    let host: String
    let folder: String
    /// The page of bia's web app to open: / is the chat, a link from bia names another.
    var path = "/"
    /// The address the title shows, when not `host` (store screenshots: the demo runs on localhost).
    var shown: String? = nil
    /// What a person calls this bia, for the file's name.
    var name = ""
    var id: String { host + path }
    private var bracketed: Bool { host.hasPrefix("[") }
    private var ipv6: Bool { !bracketed && host.filter { $0 == ":" }.count > 1 }
    var url: URL? { URL(string: (ipv6 ? "https://[\(host)]" : "https://\(host)") + (path.hasPrefix("/") ? path : "/")) }

    /// bia's page in the browser with this phone's key in the link (`#biaconfig=`): the page opens
    /// it with the password the person types and keeps a key of its own.
    func link(hash: String) -> URL? {
        guard let base = url?.absoluteString else { return nil }
        return URL(string: base + (base.contains("#") ? "&" : "#") + "biaconfig=" + hash)
    }
}

/// The folder where this app's keys wait for a browser's file picker (Files: On My iPhone → bia deploy).
func browserKeysDir() -> URL {
    FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
}

/// bia lives in the browser: this phone hands it the key. The page opens it with a password that
/// this app shows and copies; the browser then keeps a key of its own.
struct ChatPage: View {
    let target: ChatTarget
    /// unused since the chat left the app; kept for the callers
    var moved: (Int) -> Void = { _ in }
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var key: BrowserKey?
    @State private var problem: String?
    @State private var copied = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("bia opens in your browser").font(.title2.bold())
                    Text("This phone gives the browser bia's key. On bia's page, paste this password (it is copied already). After that the browser keeps a key of its own and lets you in by itself.")
                        .foregroundStyle(.secondary)
                    if let key {
                        Text(key.password)
                            .font(.system(.title3, design: .monospaced))
                            .textSelection(.enabled)
                            .padding(12)
                            .frame(maxWidth: .infinity)
                            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
                        Button(copied ? "Copied" : "Copy password") { copy(key.password) }
                        PrimaryButton(title: "Open in browser") {
                            copy(key.password)
                            if let url = target.link(hash: key.hash) { openURL(url) }
                        }
                        Text("Another browser, or this one later: on bia's sign-in page choose the file \(URL(fileURLWithPath: key.path).lastPathComponent) from Files → On My iPhone → bia deploy. It opens with the same password.")
                            .font(.footnote).foregroundStyle(.secondary)
                    } else if let problem {
                        Text(problem).foregroundStyle(.red)
                    } else {
                        ProgressView()
                    }
                }
                .padding()
            }
            .navigationTitle("bia at \(target.shown ?? target.host)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } } }
            .task {
                let t = target
                do {
                    let k = try await Task.detached {
                        try browserKey(folder: t.folder, host: t.host, name: t.name, outDir: browserKeysDir().path)
                    }.value
                    key = k
                    copy(k.password)
                } catch {
                    problem = "This phone cannot read bia's key: \(error.localizedDescription)"
                }
            }
        }
    }

    private func copy(_ s: String) {
        UIPasteboard.general.string = s
        copied = true
    }
}

extension URL: @retroactive Identifiable { public var id: String { absoluteString } }
