import SafariServices
import SwiftUI

@main
struct BiaDeployApp: App {
    @StateObject private var model = DeployModel()

    init() {
        // the core's own folder: keys, known servers, the ChatGPT registration, bia's keys
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("bia-deploy", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        `init`(appDir: dir.path)
        setBrowserOpener(opener: InAppSafari())
    }

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
    }
}

/// Pages the core opens (the ChatGPT sign-in, bia's key as a profile) talk to its listener on
/// 127.0.0.1, so they open inside this app: Safari outside it would suspend the app.
final class InAppSafari: BrowserOpener {
    func openUrl(url: String) -> Bool {
        guard let u = URL(string: url) else { return false }
        DispatchQueue.main.async { presentSafari(u) }
        return true
    }
}

@MainActor
func presentSafari(_ url: URL) {
    guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first,
          var top = scene.windows.first(where: \.isKeyWindow)?.rootViewController else { return }
    while let presented = top.presentedViewController { top = presented }
    top.present(SFSafariViewController(url: url), animated: true)
}

@MainActor
func dismissSafari() {
    guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first,
          let root = scene.windows.first(where: \.isKeyWindow)?.rootViewController,
          root.presentedViewController is SFSafariViewController else { return }
    root.dismiss(animated: true)
}
