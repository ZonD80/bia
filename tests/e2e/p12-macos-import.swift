// Import .p12 files the way Keychain Access and browsers do (SecPKCS12Import). Exit 1 unless
// every file yields an identity (key + certificate). On macOS a plain SecPKCS12Import also
// stores the key and certificates in the login keychain, so each run left a "bia agent CA"
// behind; import into memory only (macOS 15+) or into a throwaway keychain deleted at exit.
//   swiftc -O tests/e2e/p12-macos-import.swift -o p12check && PW=12345678 ./p12check a.p12 …
import Foundation
import Security
var failed = false
let pw: String = ProcessInfo.processInfo.environment["PW"] ?? ""
var opts: [String: Any] = [kSecImportExportPassphrase as String: pw]
var scratch: SecKeychain? = nil
if #available(macOS 15.0, *) {
    opts[kSecImportToMemoryOnly as String] = true
} else {
    let path = NSTemporaryDirectory() + "p12check-\(getpid()).keychain"
    let st = SecKeychainCreate(path, UInt32(pw.utf8.count), pw, false, nil, &scratch)
    guard st == errSecSuccess, let kc = scratch else {
        print("cannot create a scratch keychain: \(st)")
        exit(1)
    }
    opts[kSecImportExportKeychain as String] = kc
}
for path in CommandLine.arguments.dropFirst() {
    let data = try! Data(contentsOf: URL(fileURLWithPath: path)) as CFData
    var items: CFArray? = nil
    let st = SecPKCS12Import(data, opts as CFDictionary, &items)
    let arr = (items as? [[String: Any]]) ?? []
    let msg = (SecCopyErrorMessageString(st, nil) as String?) ?? "\(st)"
    let hasId = arr.first?[kSecImportItemIdentity as String] != nil
    print("\(path): status=\(st) \(msg) items=\(arr.count) identity=\(hasId)")
    if !hasId { failed = true }
}
if let kc = scratch {
    SecKeychainDelete(kc)
}
exit(failed ? 1 : 0)
