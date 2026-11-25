# The XMRange.dmg window, for dmgbuild (tools/build-dmgbuild.sh): the app on the left, the
# Applications shortcut on the right, assets/dmg/background.png (+ its @2x twin) behind them.
# Icon spots are icon centres in the 660x400 window, matching assets/dmg/background.html.
# Run with: -D app=<path to XMRange.app> -D background=<path to background.png>
import os

app = defines["app"]  # noqa: F821 (dmgbuild provides `defines`)
format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}
background = defines["background"]  # noqa: F821
window_rect = ((200, 120), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 128
text_size = 13
icon_locations = {os.path.basename(app): (165, 200), "Applications": (495, 200)}
