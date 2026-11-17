# Trials and profiles

Every Mac is different. So XMRange measures yours.

## Run trials

Open the **tuning** tab. The seeker card shows the button: **start trials** the first time, **continue trials** if you stopped part way, and **run trials again** when all are done. If nothing is mining, it starts mining first.

A trial is one setup, held and measured. Each one takes 8 minutes: 4 minutes of warm-up that is never scored, then 4 minutes of measuring. The card shows "trial N of M", the time left, and a time estimate for the whole run. Close other apps first for the best results.

The set of trials covers:

- every core count, from 1 up to all of them;
- the higher core counts again at background priority;
- on Apple Silicon, the efficiency cores alone.

Expect a few hours on a Mac with many cores. You can **pause**, **stop** and later **continue**. Finished trials are kept.

While trials run, your temperature and battery limits do not apply. The trials need a steady Mac.

<!-- screenshot: the tuning tab mid-trial, seeker card with its log -->

## Low Power Mode on laptops

On a laptop, the trials also run with Low Power Mode on and off, because it changes how the Mac runs CPU work. XMRange cannot switch it for you, since that needs administrator rights. So it asks, and shows "turn it on in battery settings" with **skip** and **stop** beside it.

The **low power** card on the machine tab opens a short guide: **open battery settings**, select Low Power Mode, choose Always or Only on Power Adapter. A "detected" tick shows once macOS reports it on. After both states are measured, the card shows the change in h/w for the balanced profile.

This needs macOS 12 or later. On a Mac without it, the cards are hidden.

## The profiles

When trials exist, the **profile** card offers three pills. Each shows its h/w and setup.

- **quiet:** mines on the efficiency cores at background priority. Other apps get the CPU first. On a Mac without efficiency cores, a small band of cores at background priority.
- **balanced:** about half of the cores. The rest stay free for you.
- **max:** every core, for the most hashes.

Before any trials, the pills are off and the card says "run trials first". If you set cores by hand, the card reads "manual · your own cores".

The trials card lists every measured setup, best first. A row sets that setup directly.

## Limits

Both live in settings and apply to normal mining.

- **pause above:** a CPU temperature in °F, from 100 to 230. Blank means off. Mining pauses after the CPU stays at or over it for 30 seconds. It resumes after the CPU stays a little under it for 60 seconds. The tape says "paused · over your temperature limit".
- **mine on:** **battery too** (the default) or **adapter only**. With adapter only, mining pauses on battery with "paused · on battery" and resumes when you plug in. Desktop Macs do not show this row.

If you press resume yourself during a limit pause, that overrides it until the condition clears.
