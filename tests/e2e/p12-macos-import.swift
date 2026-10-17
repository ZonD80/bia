// Import .p12 files the way Keychain Access and browsers do (SecPKCS12Import), without
// touching any keychain. Exit 1 unless every file yields an identity (key + certificate).
//   swiftc -O tools/p12-macos-import.swift -o p12check && PW=12345678 ./p12check a.p12 …
import Foundation
import Security
var failed = false
let pw: String = ProcessInfo.processInfo.environment["PW"] ?? ""
for path in CommandLine.arguments.dropFirst() {
    let data = try! Data(contentsOf: URL(fileURLWithPath: path)) as CFData
    var items: CFArray? = nil
    let opts: [String: Any] = [kSecImportExportPassphrase as String: pw]
    let st = SecPKCS12Import(data, opts as CFDictionary, &items)
    let arr = (items as? [[String: Any]]) ?? []
    let msg = (SecCopyErrorMessageString(st, nil) as String?) ?? "\(st)"
    let hasId = arr.first?[kSecImportItemIdentity as String] != nil
    print("\(path): status=\(st) \(msg) items=\(arr.count) identity=\(hasId)")
    if !hasId { failed = true }
}
exit(failed ? 1 : 0)
