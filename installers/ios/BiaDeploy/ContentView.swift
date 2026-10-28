import SwiftUI
import UniformTypeIdentifiers

private let accent = Color(red: 0.149, green: 0.361, blue: 0.627)
private let good = Color(red: 0.157, green: 0.627, blue: 0.275)

struct ContentView: View {
    @ObservedObject var model: DeployModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text("bia").font(.system(size: 34, weight: .bold)).foregroundStyle(accent)
                    Text(" deploy").font(.system(size: 34)).foregroundStyle(.secondary)
                    Spacer()
                    if model.page == .brain && !model.saved.isEmpty { Button("bia keys") { model.openKeys() } }
                }
                switch model.page {
                case .brain: BrainPage(model: model)
                case .server: ServerPage(model: model)
                case .keys: KeysPage(model: model)
                }
                if let e = model.error { ErrorCard(text: e) }
            }
            .padding(16)
            // on an iPad the wizard stays a readable column in the middle
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        .background(Color(.systemGroupedBackground))
        .scrollDismissesKeyboard(.interactively)
        #if DEBUG
        .task {
            // store screenshots: `-demo <scene>` (made-up data, no network)
            if let scene = UserDefaults.standard.string(forKey: "demo") {
                let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                    .appendingPathComponent("bia-deploy", isDirectory: true)
                model.demo(scene, appDir: dir)
            }
        }
        #endif
    }
}

struct Card<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 10) { content }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16))
    }
}

struct PrimaryButton: View {
    let title: String
    var color: Color = accent
    var enabled = true
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Text(title).font(.headline).frame(maxWidth: .infinity).frame(height: 48)
        }
        .buttonStyle(.borderedProminent)
        .tint(color)
        .clipShape(RoundedRectangle(cornerRadius: 22))
        .disabled(!enabled)
    }
}

struct ErrorCard: View {
    let text: String
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("That did not work").font(.headline)
            Text(text).font(.subheadline)
        }
        .foregroundStyle(.red)
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
    }
}

struct Busy: View {
    let text: String
    var body: some View { HStack(spacing: 10) { ProgressView(); Text(text) } }
}

struct Check: View {
    let done: Bool
    let text: String
    var body: some View {
        HStack(spacing: 8) {
            Text(done ? "✔" : "○").bold().foregroundStyle(done ? good : accent)
            Text(text).foregroundStyle(done ? .primary : .secondary)
        }
    }
}

// MARK: - brain

struct BrainPage: View {
    @ObservedObject var model: DeployModel
    @State private var help = false
    @State private var query = ""

    var body: some View {
        Card {
            Text("First, bia's brain").font(.title2.weight(.semibold))
            Text("bia thinks with a cloud AI model. Use your ChatGPT plan, or an API key from a model provider.").foregroundStyle(.secondary)
            if let email = model.chatgptEmail {
                HStack {
                    Text("Signed in to ChatGPT as \(email)")
                    Spacer()
                    Button("Sign out") { model.signOutChatGpt() }
                }
            } else {
                Button { model.signInChatGpt() } label: {
                    Text("Continue with ChatGPT").font(.headline).frame(maxWidth: .infinity).frame(height: 48)
                }
                .buttonStyle(.borderedProminent).tint(.black).clipShape(RoundedRectangle(cornerRadius: 22))
                .disabled(model.busy != nil)
                Text("Or use an API key").foregroundStyle(.secondary)
                SecureField("API key", text: $model.apiKey)
                    .textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                    .onChange(of: model.apiKey) { _ in model.keyChanged() }
                if !model.provider.isEmpty { Text("Provider: \(model.provider)").font(.subheadline).foregroundStyle(.secondary) }
                Button(help ? "Hide" : "How do I get an API key?") { help.toggle() }
                if help {
                    ForEach(keyGuides(), id: \.provider) { g in
                        VStack(alignment: .leading, spacing: 2) {
                            if let u = URL(string: g.keyUrl) { Link("\(g.name) — get a key", destination: u).bold() }
                            Text(g.about).font(.footnote).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            if !model.provider.isEmpty {
                TextField("Model", text: $model.model).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                if !model.models.isEmpty {
                    TextField("Search models", text: $query).textFieldStyle(.roundedBorder)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    let hits = model.models.filter { query.isEmpty || $0.id.localizedCaseInsensitiveContains(query) || $0.name.localizedCaseInsensitiveContains(query) }.prefix(40)
                    ScrollView {
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(Array(hits), id: \.id) { m in
                                Button { model.model = m.id; if m.ctx > 0 { model.ctx = m.ctx } } label: {
                                    Text(m.id + (m.name != m.id ? "  — \(m.name)" : "")).font(.subheadline)
                                        .frame(maxWidth: .infinity, alignment: .leading).padding(8)
                                        .background(m.id == model.model ? accent.opacity(0.15) : .clear, in: RoundedRectangle(cornerRadius: 8))
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                    .frame(maxHeight: 240)
                }
            }
        }
        Card {
            Text("bia's language").font(.title3.weight(.semibold))
            Picker("Language", selection: $model.lang) {
                ForEach(languages(), id: \.code) { l in Text(l.name + (l.voice ? "" : " (text only)")).tag(l.code) }
            }
            Text("bia talks in it from the first boot. Text only: no voice exists, so bia speaks English aloud.").font(.footnote).foregroundStyle(.secondary)
        }
        if let b = model.busy {
            Busy(text: b)
        } else if model.brainOk {
            if let h = model.hello {
                Text("bia:  \(h)").font(.title3).foregroundStyle(.white).padding(14)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(accent, in: RoundedRectangle(cornerRadius: 16))
            }
            PrimaryButton(title: "Continue  ›") { model.land = false; model.page = .server }
        } else {
            PrimaryButton(title: "Wake bia up", enabled: !model.provider.isEmpty && !model.model.isEmpty) { model.wakeBia() }
        }
        Button("No key needed: I am preparing a server that one of my bias will fly its agent to (land)") {
            model.land = true; model.page = .server
        }
        .font(.subheadline)
    }
}

// MARK: - server

struct ServerPage: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        if model.step == .login { Button("‹  Back") { model.page = .brain } }
        switch model.step {
        case .login, .connecting: LoginStep(model: model)
        case .hostKey: HostKeyStep(model: model)
        case .checking:
            Card {
                Busy(text: "Looking at the server…")
                Text("bia checks the disk, memory and network there. Nothing changes yet.").foregroundStyle(.secondary)
            }
        case .confirm: ConfirmStep(model: model)
        case .running: RunningStep(model: model)
        case .installed: InstalledStep(model: model)
        }
    }
}

struct LoginStep: View {
    @ObservedObject var model: DeployModel
    @State private var picking = false

    var body: some View {
        Card {
            Text(model.land ? "Which server will receive an agent?" : "Which server?").font(.title2.weight(.semibold))
            Text("A Linux server rented for bia (a VPS). Its provider's page shows its address and how to log in. Everything on it will be replaced by bia.").foregroundStyle(.secondary)
            TextField("Address, like 203.0.113.10", text: $model.host).textFieldStyle(.roundedBorder)
                .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
            HStack {
                TextField("User", text: $model.user).textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                TextField("Port", text: $model.port).textFieldStyle(.roundedBorder).keyboardType(.numberPad).frame(width: 80)
            }
        }
        Card {
            Text("How will bia log in?").font(.title3.weight(.semibold))
            Picker("Login", selection: $model.how) {
                ForEach(How.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            switch model.how {
            case .password:
                SecureField("The server's password", text: $model.password).textFieldStyle(.roundedBorder)
                Text("bia uses the password only now, to log in; it is not saved.").font(.footnote).foregroundStyle(.secondary)
            case .key:
                Button("Choose the key file…") { picking = true }
                    .fileImporter(isPresented: $picking, allowedContentTypes: [.data, .text, .item]) { r in
                        if case .success(let url) = r {
                            let ok = url.startAccessingSecurityScopedResource()
                            defer { if ok { url.stopAccessingSecurityScopedResource() } }
                            if let t = try? String(contentsOf: url) { model.privateKey = t }
                        }
                    }
                Text("…or paste the key (BEGIN to END):").font(.footnote)
                TextEditor(text: $model.privateKey).font(.system(size: 12, design: .monospaced)).frame(height: 90)
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color(.separator)))
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                SecureField("Passphrase (only if the key has one)", text: $model.passphrase).textFieldStyle(.roundedBorder)
            case .newKey:
                Text("When a server is ordered for bia, the provider asks for an \"SSH key\". bia makes one here: its public half goes to the provider, and bia keeps the secret half on this phone.").font(.subheadline)
                if let k = model.generated {
                    Text("1. Copy this and paste it where the provider asks for an SSH key:").bold()
                    Text(k.public).font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                        .padding(8).background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
                    HStack {
                        Button("Copy") { UIPasteboard.general.string = k.public }.buttonStyle(.bordered)
                        ShareLink(item: k.public) { Text("Share") }.buttonStyle(.bordered)
                        Button("Make another") { model.makeKey() }
                    }
                    Text("2. When the server is ready, type its address above and press Connect.").bold()
                } else {
                    PrimaryButton(title: "Make a key", enabled: model.busy == nil) { model.makeKey() }
                }
            }
        }
        if model.land { LandSecretCard(model: model) }
        if model.step == .connecting {
            Busy(text: "connecting…")
        } else {
            PrimaryButton(title: "Connect", enabled: !model.host.trimmingCharacters(in: .whitespaces).isEmpty) { model.connect() }
        }
    }
}

struct LandSecretCard: View {
    @ObservedObject var model: DeployModel
    @State private var own = false
    var body: some View {
        Card {
            Text("Land secret").font(.title3.weight(.semibold))
            Text("This server becomes an empty bia that waits for one of your bias to fly its agent there. That bia needs this secret:")
            Text(model.secret).font(.system(size: 17, weight: .semibold, design: .monospaced)).textSelection(.enabled)
            Button("Copy") { UIPasteboard.general.string = model.secret }.buttonStyle(.bordered)
            Button("Use the secret my bia made instead") { own.toggle() }.font(.subheadline)
            if own {
                TextField("Land secret", text: $model.secret).textFieldStyle(.roundedBorder).font(.system(.body, design: .monospaced))
                    .textInputAutocapitalization(.characters).autocorrectionDisabled()
            }
        }
    }
}

struct HostKeyStep: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        if let k = model.changedKey {
            Card {
                Label("This server's fingerprint changed", systemImage: "exclamationmark.triangle.fill").font(.title3.weight(.semibold)).foregroundStyle(.red)
                Text("bia knew it as \(k.known), now it says \(k.now).").font(.system(size: 12, design: .monospaced))
                Text("A server that was reinstalled gets a new fingerprint. If you did not reinstall it, stop here: someone may be in between.")
                PrimaryButton(title: "It was reinstalled: trust the new one", color: .red) { model.connect(accept: k.now) }
                Button("Back") { model.back() }
            }
        }
    }
}

private func rowName(_ k: String) -> String {
    switch k {
    case "mode": return "bia"
    case "fmm": return "brain"
    case "lang": return "language"
    case "root certificate": return "bia's key"
    case "server certificate": return "names in its certificate"
    default: return k
    }
}

struct ConfirmStep: View {
    @ObservedObject var model: DeployModel
    @State private var understood = false
    var body: some View {
        if let f = model.facts, let p = model.plan {
            Card {
                Text("\(f.hostname)  ·  \(f.os)").font(.title3.weight(.semibold))
                Text("\(f.arch == "x86_64" ? "Intel/AMD" : "ARM") server, \(String(format: "%.1f", Double(f.ramMib) / 1024)) GiB of memory\(f.rootLogin ? "" : "; bia works as root through sudo")")
                    .foregroundStyle(.secondary)
                ForEach(Array(p.rows.enumerated()), id: \.offset) { _, r in
                    HStack(alignment: .top) {
                        Text(rowName(r.what)).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
                        Text(r.value)
                    }
                    .font(.subheadline)
                }
            }
            Card {
                Text("⚠  Everything on \(f.hostname) is erased").font(.title3.weight(.semibold)).foregroundStyle(.red)
                Text("Its disk (\(p.disk)) is replaced by bia: files, websites, databases, all of it. " +
                     (p.land ? "Afterwards bia waits for one of your bias to fly its agent there with the land secret."
                             : "Afterwards you reach bia in the browser, with the key this app keeps for you."))
                Toggle("I understand, nothing on this server needs to be kept", isOn: $understood)
                PrimaryButton(title: "Replace \(f.hostname) with bia", color: .red, enabled: understood) { model.run() }
                Button("Back") { model.back() }
            }
            if p.land { LandSecretCard(model: model) }
        }
    }
}

struct RunningStep: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        Card {
            Text("Putting bia on the server").font(.title3.weight(.semibold))
            Busy(text: model.progress.isEmpty ? "starting…" : model.progress)
            if let f = model.fraction { ProgressView(value: f) }
            if !model.land { Check(done: model.keySaved, text: "bia's key (root's certificate) is saved on this phone") }
            Check(done: model.rebooting, text: "the server restarts into bia's installer")
            Text("Keep this app open. Nothing on the server changes before bia's key is safe here.").font(.footnote).foregroundStyle(.secondary)
        }
    }
}

struct InstalledStep: View {
    @ObservedObject var model: DeployModel
    @Environment(\.openURL) private var openURL

    var body: some View {
        let host = model.cleanHost
        let url = URL(string: "https://\(host.contains(":") ? "[\(host)]" : host)/")
        Card {
            switch model.up {
            case nil:
                Text("bia is installing itself").font(.title3.weight(.semibold))
                Busy(text: "The server writes bia onto its disk and starts it. This takes a few minutes… \(model.waited / 60):\(String(format: "%02d", model.waited % 60))")
            case true?:
                Label("bia is up!", systemImage: "checkmark.circle.fill").font(.title2.weight(.semibold)).foregroundStyle(good)
                if model.land {
                    Text("It waits at \(host) for your bia. Tell your bia: fly to \(host) with land secret \(model.secret)")
                } else if let u = url {
                    PrimaryButton(title: "Open bia") { openURL(u) }
                }
            case false?:
                Text("bia did not answer yet").font(.title3.weight(.semibold))
                Text(model.upError ?? "")
                if !model.land, let u = url { Button("Try to open it anyway") { openURL(u) } }
            }
        }
        if let folder = model.done?.folder { KeyCard(model: model, folder: folder, host: host) }
        if model.land { LandSecretCard(model: model) }
        Button("Set up another server") { model.again() }
    }
}

/// bia's key for one bia: install it on this phone, or share it (to a computer, another phone).
struct KeyCard: View {
    @ObservedObject var model: DeployModel
    let folder: String
    let host: String
    /// set in the list of saved keys: one short card per bia (the page explains them once)
    var savedAt: Date? = nil

    var body: some View {
        if let savedAt {
            Card {
                Text("bia at \(host)").font(.title3.weight(.semibold))
                Text("key saved \(savedAt.formatted(date: .abbreviated, time: .omitted))").font(.subheadline).foregroundStyle(.secondary)
                HStack {
                    Button("Install") { model.installProfile(folder: folder, host: host) }.buttonStyle(.bordered)
                    ShareLink(items: shareCopies(folder: folder, host: host), subject: Text("bia \(host): root's key")) {
                        Label("Share…", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.bordered)
                }
            }
        } else {
            full
        }
    }

    private var full: some View {
        Card {
            Text("bia's key").font(.title3.weight(.semibold))
            Text("bia-\(host).p12 is root's certificate: the only way into this bia. Keep a copy somewhere safe (password \(p12Password())).")
            Text("Install it on this phone: Safari offers it as a profile. Then Settings › General › VPN & Device Management installs it, and Settings › General › About › Certificate Trust Settings turns on full trust for \"bia \(host) certificate authority\".")
                .font(.subheadline)
            PrimaryButton(title: "Install bia's key") { model.installProfile(folder: folder, host: host) }
            ShareLink(items: shareCopies(folder: folder, host: host), subject: Text("bia \(host): root's key")) {
                Label("Share bia's key…", systemImage: "square.and.arrow.up")
            }
            Text("Share sends it to a computer or another phone (AirDrop, Mail, Files); it asks for the password above when it is installed.")
                .font(.footnote).foregroundStyle(.secondary)
        }
    }
}

/// Copies named after the host (bia-<host>.p12, bia-<host>-ca.crt) for the share sheet.
func shareCopies(folder: String, host: String) -> [URL] {
    let fm = FileManager.default
    let dir = fm.temporaryDirectory.appendingPathComponent("share-\(host)", isDirectory: true)
    try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
    return [("bia-root.p12", "bia-\(host).p12"), ("ca.crt", "bia-\(host)-ca.crt")].compactMap { from, to in
        let src = URL(fileURLWithPath: folder).appendingPathComponent(from)
        let dst = dir.appendingPathComponent(to)
        guard fm.fileExists(atPath: src.path) else { return nil }
        try? fm.removeItem(at: dst)
        return (try? fm.copyItem(at: src, to: dst)) != nil ? dst : nil
    }
}

/// Every bia this phone deployed: its key, to install or share again later.
struct KeysPage: View {
    @ObservedObject var model: DeployModel
    var body: some View {
        Button("‹  Back") { model.page = .brain }
        Card {
            Text("bia keys").font(.title3.weight(.semibold))
            if model.saved.isEmpty {
                Text("No bia's keys on this phone yet: they are kept here after a deploy.")
            } else {
                Text("Each is root's certificate for one bia, the only way into it (password \(p12Password())). Install offers it to this device as a profile; Share sends bia-<address>.p12 and its CA to a computer or another phone.")
                    .font(.subheadline).foregroundStyle(.secondary)
            }
        }
        ForEach(model.saved, id: \.folder) { b in
            KeyCard(model: model, folder: b.folder, host: b.host, savedAt: Date(timeIntervalSince1970: TimeInterval(b.savedAt)))
        }
    }
}
