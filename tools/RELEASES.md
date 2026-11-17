# Shipping a release

Releases go to GitHub (`House-of-Helios/xmrange`). Every Mac takes them from there, through the
app's own update pill.

1. Bump the version in `Cargo.toml`'s `[workspace.package] version`. That's the one place it lives.
   Never publish a version lower than any build a Mac may already run: the app only takes a newer
   number.
2. Run the release privacy audit on this Mac first.
3. Commit, then:
   ```
   NOTES="what changed, one line" tools/release.sh
   ```
   It refuses a dirty tree, a version already released and a signing key that doesn't match the
   app; publishes the code (`tools/publish.sh`: the filtered `public` branch); builds, zips, hashes
   and signs both apps (Apple silicon and Intel); writes `appcast.json`; creates the GitHub release
   on the `public` commit; then downloads it back and checks checksums, signatures, codesign and
   version.

## Keys (this Mac only, in `_devdata/`, never in git)

- `release-signing.key`: the Ed25519 private key (0600). **Back it up.** Without it, installed apps
  can't take another update. Its public half is `RELEASE_PUBKEY` in `crates/xmrange-core/src/update.rs`.
- `github-read-token`: while the repo is private, a fine-grained GitHub token, read-only, for this
  one repo, with an expiry date. Compiled into private-stage builds only.

## Going public

1. Delete every release built with the token (all private-stage releases) and revoke the token.
2. `gh repo edit House-of-Helios/xmrange --visibility public --accept-visibility-change-consequences`.
3. Bump the version and release again (no token now). On a second Mac, let the app take it from
   GitHub and watch it install.

## Dev builds never ship

A dev build never asks GitHub and never installs an update.
