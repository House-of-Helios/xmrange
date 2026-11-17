// Release signing for XMRange's GitHub updates (Ed25519, CryptoKit). Bookkeeper's tool, with hex
// keys and the private key read from a file, never from the command line (where `ps` could see it).
//
//   swift tools/release-sign.swift keygen <key-file>                 writes a new private key (hex, 0600),
//                                                                    prints its public key (hex)
//   swift tools/release-sign.swift public <key-file>                 prints the public key (hex)
//   swift tools/release-sign.swift sign <key-file> <version> <sha256> prints the signature (hex)
//   swift tools/release-sign.swift verify <pub-hex> <version> <sha256> <sig-hex>
//                                                                    exit 0 when it verifies
//
// The app checks the signature over "<version>\n<sha256>" with the public key in
// crates/xmrange-core/src/update.rs (RELEASE_PUBKEY), so a zip can't be relabelled as another
// version. XMRange keeps no secret in the macOS Keychain (owner ruling 2026-09-28): the private key
// is a 0600 file, _devdata/release-signing.key, on this Mac only. Back it up: without it, installed
// apps can't take another update.
import CryptoKit
import Foundation

let args = CommandLine.arguments
func die(_ s: String) -> Never { FileHandle.standardError.write(Data((s + "\n").utf8)); exit(1) }
func hex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined() }
func unhex(_ s: String) -> Data? {
    let s = s.trimmingCharacters(in: .whitespacesAndNewlines)
    guard s.count % 2 == 0 else { return nil }
    var d = Data(); var i = s.startIndex
    while i < s.endIndex {
        let j = s.index(i, offsetBy: 2)
        guard let b = UInt8(s[i..<j], radix: 16) else { return nil }
        d.append(b); i = j
    }
    return d
}
func key(_ path: String) -> Curve25519.Signing.PrivateKey {
    guard let s = try? String(contentsOfFile: path, encoding: .utf8), let d = unhex(s),
          let k = try? Curve25519.Signing.PrivateKey(rawRepresentation: d)
    else { die("not a private key file: \(path)") }
    return k
}
func message(_ version: String, _ sha: String) -> Data { Data("\(version)\n\(sha.lowercased())".utf8) }

switch args.count > 1 ? args[1] : "" {
case "keygen" where args.count == 3:
    if FileManager.default.fileExists(atPath: args[2]) { die("refusing to overwrite \(args[2])") }
    let k = Curve25519.Signing.PrivateKey()
    guard FileManager.default.createFile(atPath: args[2], contents: Data(hex(k.rawRepresentation).utf8),
                                         attributes: [.posixPermissions: 0o600])
    else { die("could not write \(args[2])") }
    print(hex(k.publicKey.rawRepresentation))
case "public" where args.count == 3:
    print(hex(key(args[2]).publicKey.rawRepresentation))
case "sign" where args.count == 5:
    guard let sig = try? key(args[2]).signature(for: message(args[3], args[4])) else { die("signing failed") }
    print(hex(sig))
case "verify" where args.count == 6:
    guard let pd = unhex(args[2]), let pub = try? Curve25519.Signing.PublicKey(rawRepresentation: pd),
          let sig = unhex(args[5]), pub.isValidSignature(sig, for: message(args[3], args[4]))
    else { die("signature does not verify") }
    print("verifies")
default:
    die("usage: release-sign.swift keygen <file> | public <file> | sign <file> <version> <sha256> | verify <pub> <version> <sha256> <sig>")
}
