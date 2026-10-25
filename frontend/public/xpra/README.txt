Xpra HTML5 client v20, pinned to https://github.com/Xpra-org/xpra-html5/releases/tag/v20
Only the browser client assets needed by index.html are included. The upstream license is MPL-2.0; see LICENSE.
Web Wireshark URL parameters additionally supply path, token, encoding=auto, reconnect=true, clipboard=false, sound=false, keyboard=false, xpramenu=false, cursor_lock_button=false, fullscreen_button=true.

Local patch: index.html init_keyboard returns early when the SimpleKeyboard library is absent (it is not vendored; the unguarded call threw and aborted page init). Do not set keyboard=true.
