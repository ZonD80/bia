#if DEBUG
import Foundation

/// Debug builds only (App Store screenshots, appstore-assets.sh): launched with `-demo <scene>`,
/// the wizard shows made-up data and no network. Scenes: brain, login, newkey, confirm,
/// running, installed, keys. Addresses are from the documentation ranges (203.0.113.0/24).
extension DeployModel {
    func demo(_ scene: String, appDir: URL) {
        let host = "203.0.113.10"
        provider = "openrouter"
        apiKey = "sk-or-v1-" + String(repeating: "0", count: 64)
        model = "deepseek/deepseek-v4.1-flash"
        hello = "Hi! I'm awake and ready. Show me my new home and I'll move in."
        markWoken()
        if scene == "brain" { return }

        page = .server
        self.host = host
        user = "root"
        how = .password
        password = "demo-password"
        if scene == "login" { return }
        if scene == "newkey" {
            self.host = ""
            how = .newKey
            generated = SshKey(public: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGs7Vb3q9wJ2dYxk4mR8pT1nZcLh0eUo5fAiQ6bXy2Wz bia-deploy", path: "")
            return
        }

        facts = Facts(hostname: "vps-frankfurt-1", os: "Ubuntu 24.04 LTS", arch: "x86_64", ramMib: 8192, rootLogin: true)
        plan = Plan(disk: "/dev/vda", land: false, rows: [
            PlanRow(what: "mode", value: "new agent"),
            PlanRow(what: "fmm", value: "openrouter deepseek/deepseek-v4.1-flash key sk-or-v1-…0000"),
            PlanRow(what: "lang", value: "English (en)"),
            PlanRow(what: "root certificate", value: "made here, kept on this phone"),
            PlanRow(what: "server certificate", value: "bia, localhost, \(host)"),
            PlanRow(what: "disk", value: "/dev/vda 160 GiB QEMU HARDDISK"),
        ])
        step = .confirm
        if scene == "confirm" { return }

        if scene == "running" {
            step = .running
            progress = "bia's system is downloading — 1.3 of 2.1 GB"
            fraction = 0.62
            keySaved = true
            return
        }

        // a few bias whose keys this phone keeps
        let servers = appDir.appendingPathComponent("servers", isDirectory: true)
        let fm = FileManager.default
        for (i, h) in [host, "198.51.100.24", "203.0.113.77"].enumerated() {
            let d = servers.appendingPathComponent(h, isDirectory: true)
            try? fm.createDirectory(at: d, withIntermediateDirectories: true)
            for f in ["bia-root.p12", "ca.crt"] {
                let p = d.appendingPathComponent(f)
                try? Data("demo".utf8).write(to: p)
                try? fm.setAttributes([.modificationDate: Date().addingTimeInterval(-Double(i) * 86_400)], ofItemAtPath: p.path)
            }
        }
        if scene == "keys" {
            page = .brain
            openKeys()
            return
        }
        keySaved = true
        rebooting = true
        done = Done(folder: servers.appendingPathComponent(host).path, caPem: "", caSha256: "", agentName: "bia-3f9c21")
        step = .installed
        up = true
    }
}
#endif
