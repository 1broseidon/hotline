---
title: Install
description: Download Toad for macOS, Windows or Linux, and what each platform needs.
---

Every build is on the [latest GitHub release](https://github.com/1Broseidon/toad/releases/latest).
Pick the file for your machine:

| Platform | File |
| --- | --- |
| macOS, Apple silicon | `…_macos_aarch64.dmg` |
| macOS, Intel | `…_macos_x86_64.dmg` |
| Windows, x86_64 | `…_windows_x86_64-setup.exe` |
| Linux, x86_64 | `.AppImage`, `.deb` or `.rpm` |

## macOS

Open the disk image and drag Toad to Applications. Builds are signed and
notarized. The menu bar is Toad's own: **Toad → Settings**, **Help → Keyboard
shortcuts**, and a Quit that actually quits.

## Windows

Run the installer. It is a per-user install, so it needs no administrator
account. The installer is not yet Authenticode-signed, so Windows may show a
SmartScreen reputation warning the first time; choose **More info → Run anyway**.

## Linux

The AppImage runs anywhere once it is executable; `.deb` and `.rpm` install
through your package manager. The tray needs `libayatana-appindicator3` at
runtime:

```bash
# Debian and Ubuntu
sudo apt install libayatana-appindicator3-1
# Fedora
sudo dnf install libayatana-appindicator-gtk3
```

Toad draws its own window frame on Linux and Windows.

## Closing the window does not quit

Closing the window hides it. The process, the teammates and their schedules
keep running. The tray icon brings the window back and is where **Quit Toad**
lives. On macOS, clicking the Dock icon also brings the window back.

This is deliberate: a teammate halfway through a build should not die because
somebody closed a window.

## Updating

A packaged Toad checks GitHub for a new version every six hours and offers it
under **Settings → Updates**. See [Updates](/setup/updates/).

## Next

[Your first teammate](/get-started/first-teammate/).
