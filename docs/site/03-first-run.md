# First run

## The notice

On first launch a short notice sits above the cards. It says two things.

> Your pool sees your wallet address, the worker name you typed and your hashrate. Tor is on, so it sees a Tor exit, not your IP address. Nothing else leaves this Mac.

> Mining pays the default wallet (GhostDrip dev wallet) until you add yours: **use yours →** on the wallet card.

If you turned Tor off, the first line says the pool sees your IP address instead.

The wallet line shows only while the default wallet is in use. The notice goes away when you press the **×**, or after you start mining.

<!-- screenshot: the first-run notice with both lines -->

## 1. Add your wallet

Find the **wallet** card on the mining tab. Click the address, or the pencil in its corner. A box opens with the hint "your monero address".

- Paste your address. The hint counts to 95 characters and says "format is correct" when it fits.
- Press **save**, or **cancel**.
- Later, **use default** returns to the default wallet.

Until you save your own address, the card reads "default · ghostdrip dev wallet". See [The default wallet](05-default-wallet.md).

<!-- screenshot: the wallet card in edit mode with a valid address hint -->

## 2. Name your worker (optional)

The **worker** card holds a name the pool shows next to your hashrate. Click it to change it. It is visible to the pool, so do not use your real name.

## 3. Choose a pool

At the top, pick **moneroocean** or **p2pool**.

- **moneroocean** needs nothing else.
- **p2pool** needs a Monero node. A guide opens from the node card: use the Monero GUI's local node, or let XMRange run its own. For its own node, open settings, set **monero node** to **xmrange**, then **choose folder** for the chain. The first sync takes a day or more.

The pool you pick applies the next time you start.

## 4. Start

On the **hashrate** card press **start**. While it runs the same button reads **pause**, and there is a **stop** beside it.

## The cards

**mining tab**
- **hashrate:** the live rate, with the run buttons.
- **h/w:** hashes per watt. In settings, **watts counted** chooses **whole mac** or **cpu only**.
- **cores:** how many cores mine. You can change the number by hand.
- **earned** and **cost:** earnings and electricity for a day, week or month. Click to change the period. Cost uses the rate you set in settings, per kWh.
- **wallet** and **worker:** as above.
- **uptime:** time since you opened the app.
- **shares:** accepted and rejected.
- **difficulty:** set by the pool. (On p2pool, **node** and **payouts** take the place of shares and difficulty.)
- **logs:** a running tape of what happens.

**machine tab:** power, chip, cpu, temperature (in °F), thermal state, memory, awake, and low power.

**tuning tab:** profile, seeker, core curve, trials, low power, watts counted. See [Trials and profiles](04-trials-and-profiles.md).

<!-- screenshot: the mining tab with all cards, mid-run -->

## Settings

The gear opens settings: electricity rate, theme, watts counted, tor, p2pool chain, monero node, xmr price, pause above, mine on, network, wallet and diagnostics. The privacy page explains the network ones.
