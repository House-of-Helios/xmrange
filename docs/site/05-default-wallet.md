# The default wallet

Until you add your own wallet, XMRange mines to the GhostDrip dev wallet. It is not an XMRange wallet. It belongs to the developer, and the app says so.

You see it three places:

- the first-run notice;
- the wallet card, which reads "default · ghostdrip dev wallet";
- settings, on the wallet row.

It exists so the app can run the moment you open it. If you leave it in place, your mining pays the developer.

## Use your own

Click the address on the wallet card, paste your Monero address, press **save**. The address is 95 characters and starts with 4 or 8. The hint tells you when the format is right.

To go back, edit the card and press **use default**.

## Where your wallet lives

Your address is plain text in the app's settings file. The pool sees it anyway. XMRange never passes it on a command line, and it does not use the macOS Keychain, so no password prompt appears.
