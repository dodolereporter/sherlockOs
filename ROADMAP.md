# Roadmap

Everything planned for SherlockOS, by milestone and by area, with its state and the ID of its
lot. A lot is one unit of work that fits in one pull request; its ID prefixes the commits
(`D-10: …`). The detailed plan, in French, is `docs/plan.md`; the decisions are in `docs/adr/`.

States:

- **Done**: merged into `main`.
- **In review**: a pull request is open, or a contributor is working on it now.
- **Planned**: not started.
- **Partly done**, **Paused**, **Open question**: said as such; an open question waits for a decision.

A milestone is reached when its exit criterion is checked, not when its lots are written.
Version 0.1 is the merge of both tracks (J10): the desktop, developed first on an existing
distribution, running on the project's own base.

| Milestone | Content | Exit criterion | State |
| --- | --- | --- | --- |
| J0 Foundations | Repository, CI, build container, QEMU bench, toolkit choice | CI green, a minimal image boots in QEMU unattended | Reached |
| J1 Nested compositor | Windows, moving, corners, shadows, XWayland | A browser and a terminal run in the compositor, in a window | Reached |
| J2 Shell | Bar, dock, launcher, notifications, component library | The desktop of the design is reproduced, animated dock included | Reached |
| J3 Real session | DRM, login, lock, portals, settings, files | A week of daily work on it without going back to another desktop | In progress |
| J4 Sherlock v1 | Daemon, consents, journal, five MCP servers, animated mascot | The "restart front-next" scenario works end to end, consent included | Started |
| J5 Bootable base | Base system, packages, image factory | The image rebuilds from scratch in CI and takes a package update | Planned |
| J6 Desktop on our base | Graphics stack, sound, network, our session by default | The desktop starts on our base in QEMU, then on real hardware | Planned |
| J7 NVIDIA, Steam, Proton | Driver, 32-bit libraries, Steam, gamepads | Three Proton games start on an NVIDIA card | Planned |
| J8 Game mode | Games app, fullscreen session, NVIDIA profile, containers paused | From boot to a game with a gamepad, without touching the keyboard | Planned |
| J9 Lab | Native Docker, sandboxes, tools, project templates | A sandbox is created, frozen and thrown away; a project template starts in one click | Planned |
| J10 0.1 | Dual-boot install, updates, hardware tests | Two weeks of daily use without going back | Planned |
| J11 Boot and installer | Themed boot menu, boot splash, graphical installer | A full install beside an existing system, without a command line | Planned |

---

## J0: foundations (reached)

| ID | Item | State |
| --- | --- | --- |
| Q-01 | Repository skeleton, rules, ADR template, pinned Rust version | Done |
| Q-02 | Rust CI: format, clippy without warnings, tests | Done |
| Q-03 | Build container of the distribution | Done |
| Q-04 | QEMU boot bench: headless boot, serial marker, screenshot | Done |
| D-00 | iced against Slint: iced chosen (ADR 0014) | Done |
| Q-07 | All development in English (ADR 0017) | Done |
| Q-08 | Language check in `just verify` and in CI, sturdier boot marker | Done |
| Q-09 | CI runner disk space | Done |
| Q-05 | Visual tests of the desktop: screenshots compared with a reference, a rendering regression fails CI | Planned |
| Q-06 | Weekly watch of package versions and security issues | Planned |

## J1: nested compositor (reached)

| ID | Item | State |
| --- | --- | --- |
| D-10 | Minimal compositor: windows, keyboard, mouse, move, resize | Done |
| D-11 | Rendering: rounded corners, shadows, spring animations | Done |
| D-12 | Layer-shell, several screens, fractional scaling | Done |
| D-13 | XWayland | Done |
| D-17 | Window handling: grab band, cursors, snapping to edges, keyboard snapping | Done |
| D-02 | GPU rendering of the shell (ADR 0018) | Done |
| D-06 | linux-dmabuf in the compositor | Done |
| D-11b | Size-only snapping; the blur and closing animation parts moved to D-34 and D-37 | Planned |

## J2: shell (reached)

| ID | Item | State |
| --- | --- | --- |
| D-01 | `sherlock-ui`: colors, radii, type, base components, logo | Done |
| D-03 | Application icons | Done |
| D-04 | Wallpapers | Done |
| D-05a, D-05b, D-05c | Light theme, switched live, automatic mode by the hour (ADR 0019) | Done |
| D-07 | Cursor theme | Done |
| D-08 | Window decorations | Done |
| D-09, D-09b | GTK and Qt themes that follow the desktop | Done |
| D-18a | Private shell protocol `sherlock-shell-v1` (ADR 0021) | Done |
| D-18w | Virtual workspaces (`ext-workspace-v1`) | Done |
| D-18b, D-18c1, D-18c2, D-18d | Working bar, maximized windows, dock on the protocol, smart hiding | Done |
| D-19, D-19b, Q-14 | Genie effect when minimizing into the dock | Done |
| D-20, D-20b, D-20c, D-20d | Top bar, control centre, calendar, notification centre, battery, Bluetooth | Done |
| D-21, D-21b | Dock with magnification, bounce, smooth add and remove, reorder by drag | Done |
| D-22a, D-22b | Launcher and search, calculator, system commands, files | Done |
| D-23 | Notifications | Done |
| D-27, E-01a, E-01b, E-01c, Q-13 | Evidence, the file manager: views, multiple selection, context menu, "Open with" | Done |
| D-31 | Touchpad gestures | Done |
| D-33 | Mission Control | Done |
| D-34 | Window opening and closing animations | Done |
| P-01 | Compositor performance | Done |
| UI-01 to UI-07 | Design system audit: animated switch, ellipsis, hover and press states, exit animations everywhere, cursors | Done |
| C-01a, C-01b | Compositor fixes: windows under the bar, larger window buttons, default wallpaper | Done |
| Q-10 | The name SherlockOS (ADR 0022) | Done |
| Q-11, Q-15 | Pointer focus after restoring, small fixes | Done |

---

## J3: real session (in progress)

### Session, security and power

| ID | Item | State |
| --- | --- | --- |
| D-14 | Real session: DRM, libseat, VT switching, entry for a display manager | Done |
| Q-12 | The session's own D-Bus | Done |
| D-35a, D-35c | Media keys with an on-screen display, simple screenshot | Done |
| D-35b | XWayland on by default, polkit agent, keyring, file portal | Done |
| D-35d | Elastic on-screen display, keyboard backlight | Done |
| D-25a, D-25b, D-25c | Secure lock: strict `ext-session-lock-v1`, PAM off the UI thread, Sherlock in 3D, rescue mode (ADR 0023) | Done |
| D-25d | Sherlock's bubble follows the theme | In review |
| D-25e | The lock comes down like a curtain | In review |
| D-25f | A hung lock program is replaced, PAM time limit, log on disk | In review |
| D-25g | Input watched while locked, a lock blind to keys is replaced | In review |
| D-25h | Lock watchdog tolerant of a slow start under heavy load | Planned |
| D-25i | Network, keyboard layout and battery at the bottom of the lock; Sherlock waves before power off | Planned |
| D-38 | Screens off like an old television, a veil before inactivity, sleep and wake | In review |
| D-43 | No lock on inactivity while a video plays (idle-inhibit) | In review |
| D-44 | XDG autostart | In review |
| - | Failed attempts limit (`pam_faillock`) for the lock, which runs with the user's rights | Open question |

### Login

| ID | Item | State |
| --- | --- | --- |
| D-24a | The compositor's greeter mode and a greetd client (ADR 0026, to approve) | In review |
| D-24b | Login screen: users, sessions (Desktop or Game mode), Sherlock | In review |
| D-24c | `just install-greeter`, arrival of the bar and the dock | In review |
| D-24d | A single bubble, Sherlock looks at the field, elements enter and leave one by one | In review |
| D-24e | The curtain: login, session and lock on one frozen picture | In review |
| D-24f | Windows reappear one by one, wallpaper from blurred to sharp, reopen the last session's applications | Planned |
| D-24g | "Switch user" from the lock screen, two sessions open | Planned |

### Bar

| ID | Item | State |
| --- | --- | --- |
| D-36a to D-36d | Global menu (DBusMenu, registrar), Sherlock menu wired to Control Room, lock and power | Done |
| D-36e | Global menu of GTK applications (`org.gtk.Menus`) | Planned |
| D-45 | Notification area behind a chevron, our own StatusNotifierWatcher and host | In review |
| D-45a | Tray icon menus, icons kept in the bar by drag and drop | In review |
| D-45b | Old X11 tray icons through `xembedsniproxy` | In review |
| Q-27 | Wi-Fi and battery icons as large as the Bluetooth one | In review |
| D-32 | Battery, brightness and power profiles: in the bar (done), in Control Room (planned) | Partly done |

### Dock

| ID | Item | State |
| --- | --- | --- |
| D-21c1 | One icon per application, window list, "New window" | Done |
| D-21e | A fullscreen window covers the bar and the dock | Done |
| D-21g | A hidden dock reserves no space | Done |
| D-21d | Several windows of one application, reveal over fullscreen, pinning by drag with a "poof" | In review |
| D-21c2 | App Expose from the dock | In review |
| D-21f | The gap below the dock is part of it (Fitts's law) | In review |
| D-21h | Sherlock moves and leaves the dock like any pinned application, the dock's own menu | In review |
| D-21i | App Expose with minimized windows and other workspaces; one dot per window, three at most | Planned |

### Launcher and applications view

| ID | Item | State |
| --- | --- | --- |
| D-41a, D-41b | Applications view: dock button, F4, search, pages, "Keep in Dock" | Done |
| P-02a, P-02b | Launcher and applications view kept resident, open in about 10 ms | Done |
| Q-21 | Launcher centered | Done |
| P-02c | Applications view after Launchpad: blurred wallpaper, centered grid, zoom in and out | In review |
| P-02d | Launcher results slide, its height and selection follow | In review |
| D-41c | Four-finger pinch opens the applications view | Planned |
| D-41d | Drag an icon from the view or the launcher to the dock | Planned |

### Compositor

| ID | Item | State |
| --- | --- | --- |
| D-29 | Clipboard between Wayland and X11: images, large data, primary selection | Done |
| Q-22 | Touchpad scrolling stops cleanly | Done |
| Q-26 | Fix of the XWayland test | Done |
| D-19d | A window raised from behind another comes forward with an animation | In review |
| D-19c | Board of the genie with the real dock | In review |
| D-37a | Blur behind glass, asked with `ext-background-effect-v1` (ADR 0027, to approve) | In review |
| D-37b | Blur everywhere translucent: notifications, on-screen display, menus, our glass windows; `org_kde_kwin_blur`; "Reduce transparency" | In review |
| D-28b | Screen capture for screen sharing, `ext-image-copy-capture-v1` (ADR 0028, to approve) | In review |
| D-28c | Window capture, cursor, dmabuf | Planned |
| D-28d | Our own screen-sharing portal, with PipeWire and our picker | Planned |
| D-29c | Drag and drop between X11 and Wayland (XDND) | Planned |
| D-35e | Screenshot v2: area, window or screen, timer, magnifier, handles, floating thumbnail | In review |
| D-42 | Clipboard history with Super+V: search, pins, privacy (password managers ignored, nothing while locked) | Planned |
| D-33c | Mission Control follow-ups from its review | Planned |
| Q-31 | Super+comma opens Control Room | Planned |
| Q-35 | Check and fix: invisible but clickable X11 windows, Super+click through the top layer | Planned |
| - | Text input method and accent input beyond dead keys | Open question |
| - | GlobalShortcuts portal (push-to-talk) and key forwarding to XWayland applications | Planned (with D-28 and J-16) |

### Evidence

| ID | Item | State |
| --- | --- | --- |
| E-04 | Path bar | In review |
| E-02 | Menus, submenus, lists and tooltips as real popups that leave the window, in all our applications | Planned |
| E-03 | Drag and drop out of and into Evidence, copy and paste of files between applications | Planned |
| D-27b | Tabs and more views | Planned |
| - | Icons from the shared icon theme, folder watching, cancelling a copy | Planned |

### Control Room

| ID | Item | State |
| --- | --- | --- |
| D-26a | Appearance, touchpad, sound, lock, default applications; every setting survives a restart | Done |
| D-26a3 | ADR 0025: display configuration through `wlr-output-management` | In review |
| D-26b | Displays page: resolution, scale, refresh rate, layout (compositor side included) | Planned |
| D-26c | Keyboard page (layout, repeat) and gesture settings | Planned |
| D-26d | "Visual effects": full, reduced, none; light mode for older PCs | Planned |
| D-26 | Network, Bluetooth, dock and bar, Sherlock, containers, games and graphics card, updates, users, about | Planned |

### Sherlock's mascot

| ID | Item | State |
| --- | --- | --- |
| D-30a to D-30d | `sherlock-3d`: glTF, wgpu, clips, gaze driven by code, one eye open, lights (ADR 0024, to approve) | Done |
| D-30e | Softer lights, the face no longer turns white | In review |
| Q-33 | The still fallback picture stays within its frame in software rendering | Planned |
| - | Seasonal variants chosen by the date | Planned |

### Hardware

| ID | Item | State |
| --- | --- | --- |
| D-15 | Explicit synchronization and direct scan-out in fullscreen | Planned |
| D-16 | Several GPUs: compositor on the integrated chip, games and external screens on the dedicated card | Planned |
| D-39 | Super+P projection picker: laptop only, duplicate, extend, second screen only | Planned |
| D-40 | NVIDIA as the main card | Planned |
| - | GPU rendering of the shell on Intel and on discrete AMD cards (software rendering today) | Planned |
| - | Performance detection for older PCs | Planned |

### Quality

| ID | Item | State |
| --- | --- | --- |
| Q-18, Q-19, Q-20, Q-23, Q-24, Q-28 | Flaky tests, hotfixes, installer copies, Flatpak applications seen, CI duration | Done |
| Q-18b | Flaky tests fixed at their cause, tests side by side in CI | In review |
| Q-29 | Tests use a made-up user name | In review |
| Q-30 | Flaky D-Bus menu test of the bar | In review |
| Q-32 | Investigate a dock panic at start without a screen | Planned |
| Q-34 | Docs for the Arch Linux base decision (ADR, plan) | Planned |
| Q-36 | Public repository: README, license, roadmap, contributing | In review |

---

## J4: Sherlock v1 (started)

| ID | Item | State |
| --- | --- | --- |
| A-10 | `sherlockd`: agent loop, MCP client, model provider interface (ADR 0015) | Done |
| A-10b | The model provider drives the real Claude Code with the user's subscription (ADR 0016) | Done |
| A-11 | Consent broker: three levels (read, act, destructive), window drawn by the compositor, append-only journal | Planned |
| A-12 | MCP server: windows | Planned |
| A-13 | MCP server: files, limited to the allowed folders | Planned |
| A-14 | MCP server: containers, on the Docker API | Planned |
| A-15 | MCP server: packages (read, plan, apply) | Planned |
| A-16 | MCP server: services and system logs | Planned |
| A-17 | MCP server: settings, network, sound | Planned |
| A-18 | MCP server: games | Planned |
| A-19 | "Ask Sherlock" panel: request, plan, consents; one window for the launcher and Sherlock on Super+Space | Planned |
| A-20 | Security review: hidden instructions in files or pages, dry runs in a sandbox | Planned |
| - | Sandboxes and notifications MCP servers | Planned |
| - | Sherlock searches the clipboard history, with consent | Planned |

## J5 and J6: our own base

The base moves to Arch Linux (decision taken, ADR to write in Q-34): our packages in our own
pacman repository on top of Arch's, the image built with archiso. The lots below are adapted
accordingly.

| ID | Item | State |
| --- | --- | --- |
| S-10 | Base system, fully scripted | Paused (reduced by the Arch decision) |
| S-11 | Kernel, initramfs, boot loader | Planned |
| S-12 | pacman, signed repository | Planned |
| S-13 | systemd, network, users, French and AZERTY by default (network service chosen by ADR) | Planned |
| S-14 | Graphics stack: Mesa, Wayland, libinput, fonts, PipeWire | Planned |
| S-15 | Image factory: disk and install key | Planned |
| S-16 | Multilib: 32-bit libraries | Planned |
| S-17 | Our desktop in packages, default session | Planned |
| S-18 | Docker Engine, Compose, cgroups v2 | Planned |
| S-22 | Laptop: sleep and wake, lid, function keys, Wi-Fi and Bluetooth | Planned |
| S-23 | Generic hardware: modular kernel, full firmware, GPU detection | Planned |
| - | Hardening: `kernel.yama.ptrace_scope=1` | Planned |

## J7 and J8: NVIDIA, Steam and game mode

| ID | Item | State |
| --- | --- | --- |
| J-10 | NVIDIA driver packages: open kernel modules, 64 and 32-bit libraries | Planned |
| J-11 | Compositor on NVIDIA: explicit sync, variable refresh rate, switching between cards | Planned |
| J-12 | Steam: package, gamepad rules, user namespaces | Planned |
| J-13 | Proton: test matrix, shader cache | Planned |
| J-14 | Games app, library: Steam files, covers, launch, gamepad navigation | Planned |
| J-15 | Games app, catalog: Steam store read-only, purchase sent to Steam | Planned |
| J-16 | Game mode session: fullscreen, notifications off, containers paused, frame limiter; voice chat overlay | Planned |
| J-17 | NVIDIA profile: driver settings, power, Proton options, measured before and after | Planned |
| J-18 | Power: the NVIDIA card sleeps outside games | Planned |

## J9: Lab

| ID | Item | State |
| --- | --- | --- |
| L-10 | Lab app: shell and navigation | Planned |
| L-11 | Containers: list, start, stop, logs | Planned |
| L-12 | Sandboxes: create, freeze, throw away, network cut | Planned |
| L-13 | Tools: Rust, Java, Node and Python, a version per project | Planned |
| L-14 | Project templates | Planned |
| L-15 | Inquiry, the terminal: segmented prompt, command blocks, project recipes, Sherlock's help on errors | Planned |

## J10 and J11: version 0.1, boot and installer

| ID | Item | State |
| --- | --- | --- |
| - | Dual-boot install, updates, hardware tests | Planned |
| S-19 | Boot menu theme | Planned |
| S-20 | Animated boot splash with Sherlock | Planned |
| S-21 | Graphical installer in six steps: language, disk, account, profile, summary, install | Planned |

## Beyond 0.1

- **Local AI.** Sherlock running on the machine, on the NPU or the GPU: offline and private.
  Today's NPUs are not there yet, so Sherlock uses the cloud meanwhile; the model provider
  stays an interface so that a local one can be added without a rewrite. A "local AI" module
  will detect the NPU, pick a model for the memory, switch between local and cloud, with a
  setting in Control Room.
- **Other model providers**, after checking their terms.
- **Postponed**: fine game tuning on AMD and Intel cards, HDR, Secure Boot, ARM, buying games
  in our interface, games with kernel anti-cheat, a home-made browser and code editor (existing
  ones are packaged).
