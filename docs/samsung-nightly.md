# Samsung nightly builds

**SUPER ALPHA.** It has never been run on a real Samsung TV - only on Samsung's TV emulator.
For Samsung TVs from 2021 or newer (Tizen 6.0 or newer) only. It may not start at all.
[If you try it, please send us a log](#send-a-log).

## What you need

- A Samsung TV running Tizen 6.0 or newer, with Developer Mode enabled. Check your model against [Samsung's TV model groups](https://developer.samsung.com/smarttv/develop/specifications/tv-model-groups.html).
- A phone or computer on the same network as the TV with [Apps2Samsung](https://github.com/Apps2Samsung/Apps2Samsung#how-it-works).
- A [nightly release](https://github.com/plxnative/plx-native-nightly/releases) with a `plxnative-tizen-v…-armv7.tpk` asset. The release notes say **No Samsung build tonight** when it is missing. Apps2Samsung may still offer an older build, so check the date in the filename.

## Install and update

1. Enable Developer Mode, enter the IP address of the computer or phone running Apps2Samsung, and restart the TV. [Samsung's Developer Mode instructions](https://developer.samsung.com/smarttv/develop/getting-started/using-sdk/tv-device.html)
2. Open Apps2Samsung and select your TV. Add `plxnative/plx-native-nightly` as a source, or select a downloaded `.tpk` to try a specific nightly. These steps follow the [Apps2Samsung instructions for custom packages](https://github.com/Apps2Samsung/Apps2Samsung#what-it-does).
3. Choose **Install** and follow the certificate prompts. Apps2Samsung signs the package for your TV. Keep using the same certificate when updating, since changing the author certificate can require uninstalling the older package.
4. Open **PlxNative Nightly** from the TV's app list. Sign in to Plex each time you start the app. The Samsung build keeps the sign-in in memory for that run.

The Apps2Samsung install and update steps still need a test with this package on a retail Samsung TV.

## Send a log

Download `tizen-log-receiver.py` and the `.tpk` from the same nightly release. On a computer on the TV's network, use Python 3.10 or newer to run the receiver:

```sh
python3 tizen-log-receiver.py --stamp PATH-TO-NIGHTLY.tpk --save samsung-nightly.log
```

The script writes a second package ending in `-with-logs.tpk` and waits for the TV's log. If your computer uses a VPN or has several network adapters, add `--address` and your computer's LAN IPv4 address. Allow inbound UDP port 9999 from your TV if the firewall asks.

Install the stamped copy with Apps2Samsung so it can sign the modified package. Keep the receiver running, open PlxNative Nightly and reproduce the problem, then press Ctrl-C on the computer. Open a [GitHub issue](https://github.com/plxnative/plx-native/issues/new) with the TV model, Tizen or firmware version, nightly filename, and what happened. Review the log for private IPs, usernames, or other identifiers before attaching it. Keep device IDs and certificates private.
