import SwiftUI

enum Page { case brain, server, keys }
enum Step { case login, connecting, hostKey, checking, confirm, running, installed }
enum How: String, CaseIterable { case password = "Password", key = "Key", newKey = "New key" }

/// The wizard: bia's brain, then the server. Every core call runs off the main thread.
@MainActor
final class DeployModel: ObservableObject {
    @Published var page = Page.brain

    // ---- brain ----
    @Published var provider = ""
    @Published var apiKey = ""
    @Published var baseUrl = ""
    @Published var model = ""
    @Published var ctx: UInt32 = 131_072
    @Published var chatgptJson: String?
    @Published var chatgptEmail: String?
    @Published var models: [CloudModel] = []
    @Published var lang = Locale.current.language.languageCode?.identifier ?? "en"
    @Published var busy: String?
    @Published var error: String?
    @Published var hello: String?
    @Published private var wokeFor: String?
    private var keyChecked = ""

    private var brainSig: String { [provider, apiKey, model, baseUrl, chatgptEmail ?? ""].joined(separator: "|") }
    var brainOk: Bool { wokeFor == brainSig }
    var brain: Brain { Brain(provider: provider, apiKey: apiKey, model: model, baseUrl: baseUrl, ctx: ctx, chatgptJson: chatgptJson) }

    private func message(_ e: Error) -> String {
        switch e as? DeployError {
        case .Failed(let m)?: return m
        case .HostKeyChanged?: return "this server's identity changed"
        case nil: return e.localizedDescription
        }
    }

    private func work(_ label: String, _ block: @escaping () async throws -> Void) {
        guard busy == nil else { return }
        busy = label
        error = nil
        Task {
            do { try await block() } catch { self.error = message(error) }
            busy = nil
        }
    }

    func keyChanged() {
        let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        guard key.count >= 20, key != keyChecked else { return }
        keyChecked = key
        work("checking the key…") {
            let info = try await Task.detached { try identifyKey(key: key) }.value
            self.provider = info.provider
            self.baseUrl = info.baseUrl
            self.models = info.models
            if self.model.isEmpty { self.model = defaultModel(provider: info.provider) }
        }
    }

    func signInChatGpt() {
        work("finish signing in on the ChatGPT page…") {
            let s = try await Task.detached { try chatgptSignIn() }.value
            dismissSafari()
            self.provider = "chatgpt"
            self.chatgptJson = s.chatgptJson
            self.chatgptEmail = s.email
            self.apiKey = ""
            self.models = s.models
            self.model = s.defaultModel ?? s.models.first?.id ?? ""
            if let c = s.models.first(where: { $0.id == self.model })?.ctx, c > 0 { self.ctx = c }
        }
    }

    func signOutChatGpt() {
        chatgptJson = nil; chatgptEmail = nil; provider = ""; model = ""; models = []
    }

    func wakeBia() {
        work("waking bia…") {
            self.hello = nil
            let b = self.brain
            let g = try await Task.detached { try wake(brain: b) }.value
            if let j = g.chatgptJson { self.chatgptJson = j }
            self.hello = g.hello
            self.wokeFor = self.brainSig
        }
    }

    // ---- server ----
    @Published var land = false
    @Published var secret = landSecret()
    @Published var host = ""
    @Published var user = "root"
    @Published var port = "22"
    @Published var how = How.password
    @Published var password = ""
    @Published var privateKey = ""
    @Published var passphrase = ""
    @Published var generated: SshKey? = savedKeys().first
    @Published var step = Step.login
    @Published var facts: Facts?
    @Published var plan: Plan?
    @Published var changedKey: (known: String, now: String)?
    @Published var progress = ""
    @Published var fraction: Float?
    @Published var keySaved = false
    @Published var rebooting = false
    @Published var done: Done?
    @Published var waited: UInt64 = 0
    @Published var up: Bool?
    @Published var upError: String?
    private var server: Server?

    var cleanHost: String {
        var h = host.trimmingCharacters(in: .whitespaces)
        if h.hasPrefix("ssh://") { h.removeFirst(6) }
        if let at = h.firstIndex(of: "@") { h = String(h[h.index(after: at)...]) }
        return h.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
    }

    private func login() throws -> Login {
        let h = host.trimmingCharacters(in: .whitespaces)
        let u = h.contains("@") ? String(h[..<h.firstIndex(of: "@")!]) : (user.isEmpty ? "root" : user)
        var key = ""
        switch how {
        case .key: key = privateKey
        case .newKey: if let g = generated { key = try readKey(path: g.path) }
        case .password: break
        }
        return Login(host: cleanHost, port: UInt16(port.trimmingCharacters(in: .whitespaces)) ?? 22, user: u,
                     password: how == .password ? password : nil,
                     privateKey: key.isEmpty ? nil : key,
                     passphrase: passphrase.isEmpty ? nil : passphrase)
    }

    private var target: Target {
        Target(host: cleanHost, brain: land ? nil : brain, lang: lang, landSecret: land ? secret.trimmingCharacters(in: .whitespaces) : nil)
    }

    private lazy var listener = Listener(model: self)

    func makeKey() {
        work("making a key…") {
            self.generated = try await Task.detached { try newKey(comment: "bia-deploy") }.value
        }
    }

    func connect(accept: String? = nil) {
        guard busy == nil else { return }
        error = nil
        changedKey = nil
        step = .connecting
        Task {
            do {
                let s = server ?? Server()
                server = s
                let l = try login()
                let t = target
                let lst = listener
                facts = try await Task.detached { try s.connect(login: l, accept: accept) }.value
                step = .checking
                plan = try await Task.detached { try s.plan(target: t, listener: lst) }.value
                step = .confirm
            } catch DeployError.HostKeyChanged(let known, let now) {
                changedKey = (known, now)
                step = .hostKey
            } catch {
                self.error = message(error)
                step = .login
            }
        }
    }

    func back() { step = .login; plan = nil }

    func run() {
        guard let s = server, let p = plan else { return }
        step = .running
        error = nil
        let t = target
        let lst = listener
        let host = cleanHost
        let isLand = land
        UIApplication.shared.isIdleTimerDisabled = true
        Task {
            defer { UIApplication.shared.isIdleTimerDisabled = false }
            do {
                let d = try await Task.detached { try s.run(target: t, disk: p.disk, listener: lst) }.value
                if chatgptJson != nil && !isLand { signOutChatGpt() } // the sign-in now belongs to that bia
                done = d
                step = .installed
                saved = savedBias()
                let ca = isLand ? nil : d.caPem
                let w = Waiter(model: self)
                try await Task.detached { try waitUp(host: host, caPem: ca, minutes: 25, listener: w) }.value
                up = true
            } catch {
                if step == .installed { up = false; upError = message(error) } else { self.error = message(error); step = .confirm }
            }
        }
    }

    // ---- bia's keys kept on this phone ----
    @Published var saved: [SavedBia] = savedBias()

    func openKeys() {
        saved = savedBias()
        page = .keys
    }

    func again() {
        step = .login; plan = nil; facts = nil; done = nil; up = nil; upError = nil
        keySaved = false; rebooting = false; progress = ""; fraction = nil
        secret = landSecret()
    }

    /// iOS takes bia's key as a configuration profile, offered from inside the app.
    func installProfile(folder: String, host: String? = nil) {
        work("preparing bia's key…") {
            let name = "bia \(host ?? self.cleanHost)"
            let url = try await Task.detached { try serveProfile(folder: folder, name: name) }.value
            if let u = URL(string: url) { presentSafari(u) }
        }
    }
}

/// The takeover's reports, back on the main thread.
final class Listener: RunListener, @unchecked Sendable {
    weak var model: DeployModel?
    init(model: DeployModel) { self.model = model }
    func step(text: String, fraction: Float?, detail: String) {
        Task { @MainActor in
            model?.progress = detail.isEmpty ? text : "\(text) — \(detail)"
            model?.fraction = fraction
        }
    }
    func line(text: String) {}
    func certificate(path: String) { Task { @MainActor in model?.keySaved = true } }
    func rebooting() { Task { @MainActor in model?.rebooting = true } }
}

final class Waiter: WaitListener, @unchecked Sendable {
    weak var model: DeployModel?
    init(model: DeployModel) { self.model = model }
    func waited(seconds: UInt64) { Task { @MainActor in model?.waited = seconds } }
}
