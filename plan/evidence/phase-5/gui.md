# Phase 5 graphical acceptance

Linux/Hyprland, 1000×970 test window, local fixture server on 127.0.0.1:8765.
Pointer events come from the explicit `shell/examples/wayland_input` protocol
client; text/keys from wtype. These act on the real shell/window/worker path.
Test-window opacity is set to 1 only for clean, window-local screenshots.

GPU acceptance, Apple M2 Pro Vulkan:

1. Click Add one repeatedly; count changes. Reset restores zero.
2. Toggle the menu; hidden content becomes visible without navigation.
3. Type a name and email, submit; the DOM displays the green valid-form result.
4. Create two Todo items using Enter and the Add button; count becomes 2 active.
   Complete one, filter Completed; only the completed title remains, struck out,
   with Undo and 1 active. Clear/filter/remove paths also pass the acceptance test.
5. The delayed timer updates its paragraph after first paint.
6. Run the infinite loop; a visible resource-stop notice appears. Escape dismisses
   it and Ctrl+L navigation successfully creates a fresh realm/page.

Software fallback acceptance repeats the same script on `--software-render`:
counter, menu, validated form, two Todos with one completed/Completed filter,
fired timer, stop notice and clean post-kill navigation. Crop regions contain
only the browser, and each saved screenshot is read back to verify content and
backend status (status bar reads `GPU` or `software` respectively).

Screenshot index (all 1000×970, verified readable):

- `gpu-counter-menu-form.png`: counter 1, menu visible, green valid-form result.
- `gpu-todo-completed.png`: Completed filter, struck-out first task, 1 active.
- `gpu-killer.png`: visible long-running-script stop notice.
- `gpu-recovered.png`: fresh `?after-kill` realm, counter reset to 0.
- `software-counter-menu-form.png`, `software-todo-completed.png`,
  `software-killer.png`, `software-recovered.png`: same states on software.

The captures predate the final keyboard-focus and native-checkpoint hardening
patches; those paths are covered by worker/layout regression tests instead
(Tab/typing/keyup/button activation, current-value control rendering).
