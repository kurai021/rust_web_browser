# Phase 7 graphical verification

Actual release window on Wayland/Hyprland, 2560×1080 output. Captures crop only
the workshop window; 1000×950 wide and 620×950 narrow. GPU adapter is Apple M2 Pro
(G14S B1), Vulkan. The forced-software run uses the same document/display list.
The server is the loopback synthetic `web_api` demo server on port 8097.

| Capture | Observation |
|---|---|
| gpu-wide.png | JSON/worker/socket/SSE results and usable masked login controls |
| gpu-login.png | Real POST login; signed-in status and hidden HttpOnly cookie |
| gpu-spa.png | pushState updates the route and omnibox to /chart without navigation |
| gpu-canvas.png | Four bars, labels, stroked axis and drawImage badge; real observer status |
| gpu-restart.png | Signed-in session restored before another login action |
| permission-clipboard.png | Origin-labelled Once / Always / Never / Deny native chrome |
| geolocation-unavailable.png | Once approval yields code 2 with no local provider |
| native-prompt.png | Per-window prompt held beyond the JS execution deadline |
| native-prompt-result.png | Answer continues the live realm; no long-script stop |
| permissions-settings.png | Current-origin/all revocation controls in about:permissions |
| software-wide.png | Persisted login in the software backend |
| software-canvas.png | Matching image/text/path chart and increased persistent visits |
| software-narrow.png | Responsive single-column form/cards without horizontal overflow |

Also exercised: Alt+Left runs the SPA popstate handler, refresh produces five
mutation records and another worker result, native fullscreen approval changes
the actual window state and Escape restores it. Revoking this origin makes the
previously remembered clipboard decision ask again. Clipboard was denied in
the desktop check; no personal clipboard content was read. The local Wayland
clipboard and notification executables are installed; their error/consent paths
are explicit rather than fabricated success.

The two UI findings were fixed before final gates: percentage-width input
controls preserve their intrinsic height, and native prompt wait time is removed
from elapsed VM execution accounting while instruction/call/cancel caps remain.
The input mask and modal-budget behaviors have regression tests. Narrow fixture
tracks use standard minmax(0,1fr) sizing for responsive canvas content.

The environment emits an existing XCompose include warning; it did not prevent
the tested ASCII input, shortcuts, native replies or painting. The optional
wayland_input example and wtype are test tools, not product input backends.
