# Elearnin’

Elearnin’ downloads study materials from BUE Learn1 into a local library. Open the app, choose a module, then click a lecture or tutorial to open the saved file in your default viewer.

Sync opens course sections and Moodle week tiles in the background, including downloadable files in linked Moodle folders. The module view has **Lectures**, **Tutorials**, and **Everything else** columns. Files already saved on your device open while sync runs. The header includes a theme toggle, and the mouse Back button returns from a module to the module grid.

## Run it

Install a current stable Rust toolchain. On Windows, install Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK. On Linux, install a C compiler, `pkg-config`, D-Bus development headers, a running Secret Service keyring such as GNOME Keyring or KWallet with Secret Service enabled, and `xdg-open`.

```sh
cargo run --release --locked
```

On the first launch, click **Connect BUE**, enter your Learn1 username and password, then click **Save and sync**. Enter credentials directly in the app. Elearnin’ stores them in Windows Credential Manager or the Linux desktop keyring. **Forget sign-in** removes them and leaves the saved library available.

Elearnin’ syncs on launch and every 15 minutes while it’s open. It uses Moodle’s mobile API when available and can fall back to the ordinary Learn1 login and course pages. If the session expires, it signs in again with the saved credentials. If the university rejects the credentials or requires an interactive check, the sync status explains what happened.

Each device has its own offline copy. The first version syncs downloadable course files. External streams and activities such as quizzes or submissions are handled through the university portal. Android packaging, credential storage, and the Android file viewer handoff are future work.

## Windows installer

The x64 installer runs on Windows 10 and 11. It installs for the current user, adds a Start menu shortcut, and offers an optional desktop shortcut. Uninstall through **Settings → Apps → Installed apps**. Your offline library and saved sign-in remain available for a reinstall. Use **Forget sign-in** in Elearnin’ to remove the stored credentials.

To build the installer, install Inno Setup 6.3 or newer and run this command in PowerShell from the project folder. Pass the full path to `ISCC.exe` if it is outside your PATH.

```powershell
.\packaging\windows\build-installer.ps1 -InnoCompiler 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
```

The installer and its SHA-256 checksum go into `target/installer`.

## Install on Linux

Install the package for your distribution, then open **Elearnin’** from the applications menu. The packages target x86-64 computers. The Debian package is built on Debian 12 for use on Debian 12 or newer and compatible Mint versions.

```sh
# Debian / Linux Mint
sudo apt install ./elearnin_1.0.0_amd64.deb

# Arch Linux
sudo pacman -U ./elearnin-1.0.0-1-x86_64.pkg.tar.zst
```

Automatic sign-in uses the desktop's Secret Service keyring. GNOME Keyring and KeePassXC with Secret Service enabled are supported options. Package removal leaves your downloads and saved sign-in in your user account.

## Verify locally

```sh
cargo test --locked
```

For a live check, let one module sync, close and reopen the app to start a fresh session, and confirm it syncs without prompting. Then disconnect from the internet and open a downloaded lecture from its module.
