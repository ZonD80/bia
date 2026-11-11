import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// One bia this phone holds the key of. `host` is what is dialled, a port included.
struct ChatTarget: Identifiable {
    let host: String
    let folder: String
    /// The page of bia's web app to open: / is the chat, a link from bia names another.
    var path = "/"
    /// The address the title shows, when not `host` (store screenshots: the demo chat runs on localhost).
    var shown: String? = nil
    var id: String { host + path }
    private var bracketed: Bool { host.hasPrefix("[") }
    private var ipv6: Bool { !bracketed && host.filter { $0 == ":" }.count > 1 }
    /// The address without a port, as requests report it.
    var name: String {
        if bracketed { return String(host.dropFirst().prefix { $0 != "]" }) }
        return ipv6 ? host : String(host.split(separator: ":").first ?? "")
    }
    var url: URL? { URL(string: (ipv6 ? "https://[\(host)]" : "https://\(host)") + (path.hasPrefix("/") ? path : "/")) }
    func isHere(_ h: String?) -> Bool { h?.trimmingCharacters(in: CharacterSet(charactersIn: "[]")) == name }
}

/// bia's own CA and root's certificate, as kept in its folder on this phone.
private struct ChatKeys {
    let ca: SecCertificate
    let identity: SecIdentity

    init(folder: String) throws {
        let dir = URL(fileURLWithPath: folder)
        let pem = try String(contentsOf: dir.appendingPathComponent("ca.crt"), encoding: .utf8)
        let body = pem.split(separator: "\n").filter { !$0.hasPrefix("-----") }.joined()
        guard let der = Data(base64Encoded: body), let ca = SecCertificateCreateWithData(nil, der as CFData) else {
            throw ChatError.message("bia's certificate authority cannot be read")
        }
        self.ca = ca
        let p12 = try Data(contentsOf: dir.appendingPathComponent("bia-root.p12"))
        var items: CFArray?
        let status = SecPKCS12Import(p12 as CFData, [kSecImportExportPassphrase as String: p12Password()] as CFDictionary, &items)
        guard status == errSecSuccess, let first = (items as? [[String: Any]])?.first,
              let value = first[kSecImportItemIdentity as String] else {
            throw ChatError.message("bia's key cannot be read (\(status))")
        }
        identity = value as! SecIdentity
    }
}

private enum ChatError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case .message(let m) = self { return m } else { return nil } }
}

/// bia's web app inside this app: root's key and bia's CA come from this phone, nothing to install.
struct ChatPage: View {
    let target: ChatTarget
    @Environment(\.dismiss) private var dismiss
    @State private var problem: String?
    @State private var reload = 0
    @State private var saved: URL?

    var body: some View {
        NavigationStack {
            Group {
                if let keys = try? ChatKeys(folder: target.folder) {
                    ChatWeb(target: target, keys: keys, reload: reload, problem: $problem, saved: $saved)
                        .ignoresSafeArea(.container, edges: .bottom)
                } else {
                    Text("This phone cannot read bia's key.").padding()
                }
            }
            .overlay(alignment: .top) {
                if let problem { Text(problem).padding(12).background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12)).padding() }
            }
            .navigationTitle("bia at \(target.shown ?? target.host)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                ToolbarItem(placement: .primaryAction) { Button("Reload") { problem = nil; reload += 1 } }
            }
            .sheet(item: $saved) { ShareFile(url: $0) }
        }
    }
}

extension URL: @retroactive Identifiable { public var id: String { absoluteString } }

private struct ShareFile: UIViewControllerRepresentable {
    let url: URL
    func makeUIViewController(context: Context) -> UIActivityViewController { UIActivityViewController(activityItems: [url], applicationActivities: nil) }
    func updateUIViewController(_: UIActivityViewController, context: Context) {}
}

private struct ChatWeb: UIViewRepresentable {
    let target: ChatTarget
    let keys: ChatKeys
    let reload: Int
    @Binding var problem: String?
    @Binding var saved: URL?

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.uiDelegate = context.coordinator
        web.isOpaque = false // no white flash in dark mode before bia's page has drawn
        web.backgroundColor = .systemBackground
        #if DEBUG
        if #available(iOS 16.4, *) { web.isInspectable = true }
        #endif
        if let url = target.url { web.load(URLRequest(url: url)) }
        return web
    }

    func updateUIView(_ web: WKWebView, context: Context) {
        if context.coordinator.reloads != reload {
            context.coordinator.reloads = reload
            web.reload()
        }
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
        let parent: ChatWeb
        var reloads = 0
        private var downloads: [ObjectIdentifier: URL] = [:]

        init(_ parent: ChatWeb) { self.parent = parent }

        private func answer(_ challenge: URLAuthenticationChallenge) async -> (URLSession.AuthChallengeDisposition, URLCredential?) {
            let space = challenge.protectionSpace
            guard parent.target.isHere(space.host) else { return (.performDefaultHandling, nil) }
            switch space.authenticationMethod {
            case NSURLAuthenticationMethodServerTrust:
                // this bia's CA alone: the system's own list means nothing here. The host name is still checked.
                guard let trust = space.serverTrust else { return (.cancelAuthenticationChallenge, nil) }
                SecTrustSetAnchorCertificates(trust, [parent.keys.ca] as CFArray)
                SecTrustSetAnchorCertificatesOnly(trust, true)
                var error: CFError?
                if SecTrustEvaluateWithError(trust, &error) { return (.useCredential, URLCredential(trust: trust)) }
                let why = error.map { CFErrorCopyDescription($0) as String? ?? "" } ?? ""
                NSLog("bia chat: server trust refused: %@", why)
                await MainActor.run { parent.problem = "bia's certificate was refused, so the chat stayed closed. \(why)" }
                return (.cancelAuthenticationChallenge, nil)
            case NSURLAuthenticationMethodClientCertificate:
                return (.useCredential, URLCredential(identity: parent.keys.identity, certificates: nil, persistence: .forSession))
            default:
                return (.performDefaultHandling, nil)
            }
        }

        func webView(_ web: WKWebView, didReceive challenge: URLAuthenticationChallenge, completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
            Task {
                let (d, c) = await self.answer(challenge)
                completionHandler(d, c)
            }
        }

        #if DEBUG
        // for testing without a finger: `-chatjs <script>` runs once the page is up
        func webView(_ web: WKWebView, didFinish navigation: WKNavigation!) {
            guard let js = UserDefaults.standard.string(forKey: "chatjs") else { return }
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                web.callAsyncJavaScript(js, arguments: [:], in: nil, in: .page) { result in NSLog("bia chat js: %@", String(describing: result)) }
            }
        }
        #endif

        func webView(_ web: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            if parent.problem == nil { parent.problem = "bia did not answer at \(parent.target.host): \(error.localizedDescription). Reload tries again." }
        }

        func webView(_ web: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            if action.shouldPerformDownload { decisionHandler(.download); return }
            if let url = action.request.url, url.scheme == "https" || url.scheme == "http", !parent.target.isHere(url.host) {
                UIApplication.shared.open(url)
                decisionHandler(.cancel)
                return
            }
            decisionHandler(.allow)
        }

        func webView(_ web: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
            decisionHandler(response.canShowMIMEType ? .allow : .download)
        }

        func webView(_ web: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) { download.delegate = self }
        func webView(_ web: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) { download.delegate = self }

        func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String, completionHandler: @escaping (URL?) -> Void) {
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("bia-chat-\(UUID().uuidString)", isDirectory: true)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let file = dir.appendingPathComponent(suggestedFilename)
            downloads[ObjectIdentifier(download)] = file
            completionHandler(file)
        }

        func downloadDidFinish(_ download: WKDownload) {
            if let file = downloads.removeValue(forKey: ObjectIdentifier(download)) { parent.saved = file }
        }

        // a link that opens a new tab stays here, or leaves for the browser above
        func webView(_ web: WKWebView, createWebViewWith config: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if action.targetFrame == nil { web.load(action.request) }
            return nil
        }

        // the system asks the person once; only bia's own page may ask at all
        func webView(_ web: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin, initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType, decisionHandler: @escaping (WKPermissionDecision) -> Void) {
            decisionHandler(parent.target.isHere(origin.host) && type == .microphone ? .grant : .deny)
        }

        func webView(_ web: WKWebView, runJavaScriptAlertPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping () -> Void) {
            present(message, ok: { completionHandler() }, cancel: nil)
        }

        func webView(_ web: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (Bool) -> Void) {
            present(message, ok: { completionHandler(true) }, cancel: { completionHandler(false) })
        }

        private func present(_ message: String, ok: @escaping () -> Void, cancel: (() -> Void)?) {
            guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first,
                  var top = scene.windows.first(where: \.isKeyWindow)?.rootViewController else { ok(); return }
            while let p = top.presentedViewController { top = p }
            let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in ok() })
            if let cancel { alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in cancel() }) }
            top.present(alert, animated: true)
        }
    }
}
