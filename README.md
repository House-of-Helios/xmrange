# XMRange

XMRange is a Mac app that mines Monero (XMR) on the CPU. Its goal is the most hashes per watt that the Mac can give. It runs on Apple Silicon Macs and, as a separate app, on Intel Macs. It keeps everything on the Mac and needs no administrator password.

- **Measures and does not guess.** "Run trials" measures every core count and utility priority. On laptops, it also measures with and without Low Power Mode. The results are saved on the Mac.
- **Three profiles from the trials:**
  - **Quiet:** a few cores at background priority (the efficiency cores on Apple Silicon).
  - **Balanced:** about half of the cores, at the best hashes per watt.
  - **Max:** every core.
- **Two pools:**
  - **MoneroOcean.** Simple to start.
  - **p2pool.** No pool operator. It uses a Monero node: the one the Monero GUI runs, or a node that XMRange runs itself. That node's chain goes in a folder you choose, and an external drive is fine.
- **Tor is on by default.** The bundled Tor client is Arti, from the Tor Project. With Tor on, the pool or the p2pool peers see a Tor exit, not your IP address.
- **Limits.** Mining can pause above a CPU temperature, or while the Mac is on battery.

## What leaves your Mac

XMRange connects only to the pool that you select. It does not use both pools at the same time.

| You choose | XMRange connects to | They see |
|---|---|---|
| MoneroOcean | the MoneroOcean pool | your wallet address, your worker name, your hashrate. They see your IP address only when Tor is off. |
| p2pool | p2pool peers | your shares. Your payout address is public on the p2pool share chain. They see your IP address only when Tor is off. |
| p2pool with XMRange's own node | p2pool peers and Monero peers | the same as above. Monero peers see that you run a node. |
| Tor on (the default) | the Tor network | — |
| XMR price "fetch" (off by default) | Kraken, then Yahoo Finance, through Tor | one price request each hour |
| Problem report (only when you press send) | Nostr relays `nos.lol`, `relay.primal.net`, `relay.snort.social`, `nostr.mom`, through Tor | a message sealed for XMRange's key: the version, your note and scrubbed diagnostics, with no wallet, no IP address and no names. You read and can edit all of it first. |
| Update checks | GitHub, through Tor | a request for the latest XMRange release, at launch and every 6 hours. Nothing about you or your mining. |

These things never leave your Mac:
- telemetry;
- crash reports.

Settings → network → "show connections" lists the live connections, as counts and ports only.

At rest:
- Your IP address is not saved.
- The trials database is encrypted.
- Your wallet is in the settings file, where the pool can see it anyway.

## Install

1. Download the zip for your Mac from Releases: `XMRange-<version>-apple-silicon.zip` or `XMRange-<version>-intel.zip`. Each zip has a `.sha256` file beside it.
2. Unzip it and move `XMRange.app` to Applications.
3. The app is not signed by Apple, so macOS stops the first open:
   - **macOS 15 and later:** open the app once. Then open System Settings → Privacy & Security and click "Open Anyway".
   - **macOS 14 and earlier:** Control-click the app and select Open.

**Requirements:**
- Apple Silicon: macOS 11.3 or later.
- Intel: macOS 10.15 or later.
- The Low Power Mode guidance needs macOS 12 or later.
- About 2.5 GB of free memory while mining.
- XMRange's own Monero node needs about 140 GB on an APFS or Mac OS Extended drive.

## Build from source

You need the Xcode command line tools, CMake and Rust (through `rustup`, with the `x86_64-apple-darwin` target for the Intel app).

```sh
tools/build-xmrig.sh      # xmrig (MoneroOcean fork) at a pinned commit, with patches/xmrig-mo-xmrange.patch
tools/build-p2pool.sh     # p2pool at a pinned tag
tools/build-arti.sh       # Arti (Tor) at a pinned tag
tools/build-monerod.sh    # monerod at a pinned tag, dependencies built from source (contrib/depends)
tools/build-x86_64.sh     # the same helpers for Intel Macs
tools/build-release-app.sh              # XMRange.app for Apple Silicon
ARCH=x86_64 tools/build-release-app.sh  # XMRange.app for Intel Macs
```

Each helper is built from its own upstream source, at a pinned version, with only system libraries.

The xmrig patch does three things:
- it sets the developer donation to 0;
- an API reload never rewrites the config file;
- a pool that names its algorithm gets only that algorithm in the login (p2pool refuses longer logins).

`cargo devtest` runs the tests. `tools/check-release.sh` proves that a release build carries no developer paths, no dev-only code and no wallet other than the disclosed default.

## The default wallet

Until you add your own wallet, mining pays the GhostDrip developer wallet. The app says this on first start and on the wallet card. Add your own wallet on the wallet card.

## License

GPL-3.0-or-later (`LICENSE`). The bundled xmrig and p2pool are GPL-3.0. Arti is MIT/Apache-2.0. monerod uses the Monero license (BSD-3-Clause).
